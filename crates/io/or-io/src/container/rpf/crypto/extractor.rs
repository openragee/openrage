// Sliding-window SHA-1 scan over `gta5.exe` to recover the AES key, NG
// key blocks, NG decrypt tables, hash LUT and AWC key.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

#[cfg(feature = "rayon")]
use rayon::prelude::*;
use sha1::{Digest, Sha1};

use crate::error::{IoError, Result};

use super::hashes::{
    PC_AES_KEY_HASH, PC_AES_KEY_PAYLOAD_LEN, PC_LUT_HASH, PC_LUT_PAYLOAD_LEN,
    PC_NG_DECRYPT_TABLE_HASHES, PC_NG_DECRYPT_TABLE_PAYLOAD_LEN, PC_NG_KEY_HASHES,
    PC_NG_KEY_PAYLOAD_LEN,
};
use super::keys::{
    Gta5Keys, NgDecryptTableRound, AWC_KEY_WORDS, GTA5_HASH_LUT_LEN, NG_DECRYPT_TABLE_BYTES,
    NG_DECRYPT_TABLE_LANES, NG_DECRYPT_TABLE_ROUNDS, NG_KEY_COUNT, NG_KEY_LEN, PC_AES_KEY_LEN,
};
use super::provenance::{ExeVerdict, KeyOrigin, KeyProvenance, SnapshotAgreement};
use super::spread;

const ALIGN_LENGTH: usize = 8;

const AWC_KEY_BYTES: usize = AWC_KEY_WORDS * 4;

/// SHA-1 of the 16-byte AWC key.
///
/// ⚠ Ours rather than ported with the rest of the fingerprint table: the key
/// sits verbatim and 8-aligned in every Legacy and Enhanced executable
/// `what_the_extractor_recovers_per_executable` scanned, and that suite checks
/// this digest against a user snapshot's key wherever one is supplied.
const PC_AWC_KEY_HASH: [u8; 20] = [
    0xC3, 0x1E, 0x0D, 0xA1, 0xC7, 0xE5, 0xD8, 0x55, 0x02, 0xE5, 0x17, 0x29, 0x0E, 0xD0, 0x7B, 0x18,
    0x0A, 0x94, 0xE9, 0x11,
];

/// Which key snapshot the NG fallback decodes, if any.
///
/// ⚠ A snapshot is the **only** source of NG keys against every retail build
/// this project can point at. `File` is the user's own; `Bundled` is the one
/// `bundled-keys` embeds (D18); `None` is the executable alone, which recovers
/// the AES key, the hash LUT and the AWC key and refuses NG-encrypted archives
/// by name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SnapshotSource {
    #[default]
    None,
    File(std::path::PathBuf),
    #[cfg(feature = "bundled-keys")]
    Bundled,
}

/// The file name a user-supplied key snapshot is looked for under, beside the
/// keys cache.
pub const USER_SNAPSHOT_FILE_NAME: &str = "magic.dat";

/// What a key set without NG keys cannot open, for every message that has to say so.
#[cfg(feature = "bundled-keys")]
pub const WITHOUT_A_SNAPSHOT: &str = "AES-encrypted archives and scrambled .awc audio open \
     from your executable alone. NG-encrypted archives need NG keys from a key snapshot, and \
     these keys have none: they were extracted in AES-only mode, before OpenRage bundled a \
     snapshot, or with a snapshot that describes another build. Extract the keys again; to use \
     your own key snapshot, place magic.dat beside the keys cache first or pass one with --magic";
/// What a key set without NG keys cannot open, for every message that has to say so.
#[cfg(not(feature = "bundled-keys"))]
pub const WITHOUT_A_SNAPSHOT: &str = "AES-encrypted archives and scrambled .awc audio open \
     from your executable alone. NG-encrypted archives need NG keys that no retail executable \
     carries and this build does not bundle; place your own key snapshot (magic.dat) beside the \
     keys cache, or pass one with --magic, to open them";

/// Names a key snapshot for the default extraction path, which is what the
/// install-gated suites and the shader tool reach.
pub const SNAPSHOT_ENV: &str = "OPENRAGE_KEY_SNAPSHOT";

impl SnapshotSource {
    /// The snapshot `OPENRAGE_KEY_SNAPSHOT` names, if it names a file.
    pub fn from_env() -> Self {
        match std::env::var_os(SNAPSHOT_ENV).map(std::path::PathBuf::from) {
            Some(path) if path.is_file() => SnapshotSource::File(path),
            _ => SnapshotSource::None,
        }
    }

    /// The user's own snapshot if `config_dir` holds one.
    pub fn discover(config_dir: Option<&Path>) -> Self {
        match config_dir.map(|dir| dir.join(USER_SNAPSHOT_FILE_NAME)) {
            Some(path) if path.is_file() => SnapshotSource::File(path),
            _ => SnapshotSource::None,
        }
    }

    /// This source, or the snapshot `bundled-keys` embeds when it names none.
    pub fn or_bundled(self) -> Self {
        #[cfg(feature = "bundled-keys")]
        if self == SnapshotSource::None {
            return SnapshotSource::Bundled;
        }
        self
    }

    /// Whether extracting again with this source could supply the NG keys `cache` lacks.
    // ⚠ Only a cache an earlier build wrote matches, so a re-extraction that
    // still yields no NG keys is not repeated on the next start.
    pub fn would_complete(&self, cache: &super::cache::KeysCache) -> bool {
        *self != SnapshotSource::None && cache.predates_the_bundled_snapshot()
    }

    /// The snapshots to try, in order: a user's file falls back to the bundled one.
    fn candidates(&self) -> Vec<SnapshotSource> {
        match self {
            SnapshotSource::None => Vec::new(),
            #[cfg(feature = "bundled-keys")]
            SnapshotSource::File(_) => vec![self.clone(), SnapshotSource::Bundled],
            _ => vec![self.clone()],
        }
    }

    fn origin(&self) -> KeyOrigin {
        match self {
            #[cfg(feature = "bundled-keys")]
            SnapshotSource::Bundled => KeyOrigin::EmbeddedSnapshot,
            _ => KeyOrigin::SnapshotFile,
        }
    }

    pub fn describe_for_user(&self) -> String {
        self.describe()
    }

    fn describe(&self) -> String {
        match self {
            SnapshotSource::None => "no key snapshot".into(),
            SnapshotSource::File(path) => format!("the key snapshot at {}", path.display()),
            #[cfg(feature = "bundled-keys")]
            SnapshotSource::Bundled => "the key snapshot bundled with OpenRage".into(),
        }
    }

    fn decode(&self, aes_key: &[u8; PC_AES_KEY_LEN]) -> Option<Result<Gta5Keys>> {
        match self {
            SnapshotSource::None => None,
            SnapshotSource::File(path) => Some(super::magic::decode_magic_file(path, aes_key)),
            #[cfg(feature = "bundled-keys")]
            SnapshotSource::Bundled => Some(super::magic::decode_magic_blob(
                super::magic::BUNDLED_SNAPSHOT,
                aes_key,
            )),
        }
    }
}

pub type ProgressCb<'a> = dyn Fn(ExtractionProgress) + Send + Sync + 'a;

// Coarse extraction progress, reported per payload bucket.
#[derive(Debug, Clone)]
pub struct ExtractionProgress {
    pub stage: &'static str,
    pub found: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExeFingerprint {
    pub sha1_hex: String,
    pub size: u64,
}

pub fn fingerprint_executable<P: AsRef<Path>>(path: P) -> Result<ExeFingerprint> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|e| IoError::FileReadFailed {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    let sha1 = hasher.finalize();
    let mut hex = String::with_capacity(40);
    for byte in sha1 {
        hex.push_str(&format!("{:02x}", byte));
    }
    Ok(ExeFingerprint {
        sha1_hex: hex,
        size: bytes.len() as u64,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionMode {
    // Everything: the AES key, LUT and AWC key, and the NG material from a
    // snapshot where the executable lacks it.
    Full,
    // The AES key, LUT and AWC key; skips the NG sweep and consults no
    // snapshot.
    AesOnly,
}

pub fn extract_keys<P: AsRef<Path>>(exe_path: P, progress: &ProgressCb<'_>) -> Result<Gta5Keys> {
    extract_keys_with(exe_path, ExtractionMode::Full, progress)
}

/// The AES key in a Rockstar executable, which RDR2's carries as GTA V's does.
pub fn aes_key_from_executable<P: AsRef<Path>>(exe_path: P) -> Result<[u8; PC_AES_KEY_LEN]> {
    let path = exe_path.as_ref();
    let file = std::fs::File::open(path).map_err(|e| IoError::FileReadFailed {
        path: path.to_path_buf(),
        source: e,
    })?;
    let exe = executable_bytes(&file)?;
    let found = scan_one(&exe, &PC_AES_KEY_HASH, PC_AES_KEY_PAYLOAD_LEN)
        .ok_or_else(|| IoError::KeyExtractionFailed(format!("no AES key in {}", path.display())))?;
    found
        .try_into()
        .map_err(|_| IoError::KeyExtractionFailed("AES payload length mismatch".into()))
}

// Extract the AES key from `exe_path` and use it to decode a CodeWalker
// `magic.dat` blob, which supplies the NG keys, decrypt tables and LUT.
pub fn extract_keys_from_magic<P: AsRef<Path>, M: AsRef<Path>>(
    exe_path: P,
    magic_path: M,
    progress: &ProgressCb<'_>,
) -> Result<Gta5Keys> {
    let aes_only = extract_keys_with(exe_path, ExtractionMode::AesOnly, progress)?;
    progress(ExtractionProgress {
        stage: "magic.dat decode",
        found: 0,
        total: 1,
    });
    let snapshot = SnapshotSource::File(magic_path.as_ref().to_path_buf());
    let mut keys = super::magic::decode_magic_file(magic_path.as_ref(), &aes_only.aes_key)?;
    keys.provenance = KeyProvenance {
        aes_key: KeyOrigin::Executable,
        lut: KeyOrigin::SnapshotFile,
        ng_keys: KeyOrigin::SnapshotFile,
        ng_decrypt_tables: KeyOrigin::SnapshotFile,
        awc_key: KeyOrigin::SnapshotFile,
        snapshot_agreement: SnapshotAgreement::Unchecked,
    };
    reconcile_snapshot_against_executable(&mut keys, Some(&aes_only.lut), &snapshot);
    if aes_only.provenance.awc_key == KeyOrigin::Executable {
        keys.awc_key = aes_only.awc_key;
        keys.provenance.awc_key = KeyOrigin::Executable;
    }
    progress(ExtractionProgress {
        stage: "magic.dat decode",
        found: 1,
        total: 1,
    });
    Ok(keys)
}

pub fn extract_keys_with<P: AsRef<Path>>(
    exe_path: P,
    mode: ExtractionMode,
    progress: &ProgressCb<'_>,
) -> Result<Gta5Keys> {
    extract_keys_with_snapshot(
        exe_path,
        mode,
        &SnapshotSource::from_env().or_bundled(),
        progress,
    )
}

pub fn extract_keys_with_snapshot<P: AsRef<Path>>(
    exe_path: P,
    mode: ExtractionMode,
    snapshot: &SnapshotSource,
    progress: &ProgressCb<'_>,
) -> Result<Gta5Keys> {
    let path = exe_path.as_ref();
    let file = std::fs::File::open(path).map_err(|e| IoError::FileReadFailed {
        path: path.to_path_buf(),
        source: e,
    })?;
    let exe = executable_bytes(&file)?;

    progress(ExtractionProgress {
        stage: "AES + LUT",
        found: 0,
        total: 2,
    });

    let aes_payload = scan_one(&exe, &PC_AES_KEY_HASH, PC_AES_KEY_PAYLOAD_LEN);
    progress(ExtractionProgress {
        stage: "AES + LUT",
        found: usize::from(aes_payload.is_some()),
        total: 2,
    });

    let lut_payload = scan_one(&exe, &PC_LUT_HASH, PC_LUT_PAYLOAD_LEN);
    progress(ExtractionProgress {
        stage: "AES + LUT",
        found: usize::from(aes_payload.is_some()) + usize::from(lut_payload.is_some()),
        total: 2,
    });
    let awc_payload = scan_one(&exe, &PC_AWC_KEY_HASH, AWC_KEY_BYTES);

    let Some(aes_payload) = aes_payload else {
        let verdict = ExeVerdict::of(&exe, lut_payload.is_some());
        return Err(IoError::KeyExtractionFailed(format!(
            "no AES key in {}: {}.\n  {}",
            path.display(),
            verdict.cause(),
            verdict.remedy()
        )));
    };

    if mode == ExtractionMode::AesOnly {
        let lut_payload = lut_payload.ok_or_else(|| {
            IoError::KeyExtractionFailed(
                "hash LUT not found in the executable, and AES-only mode does not consult a \
                 key snapshot. Re-run without --aes-only so the LUT comes from one."
                    .into(),
            )
        })?;
        let mut keys = Gta5Keys::empty();
        if aes_payload.len() != PC_AES_KEY_LEN || lut_payload.len() != GTA5_HASH_LUT_LEN {
            return Err(IoError::KeyExtractionFailed(
                "AES/LUT payload length mismatch".into(),
            ));
        }
        keys.aes_key.copy_from_slice(&aes_payload);
        keys.lut.copy_from_slice(&lut_payload);
        keys.provenance = KeyProvenance {
            aes_key: KeyOrigin::Executable,
            lut: KeyOrigin::Executable,
            ng_keys: KeyOrigin::Absent,
            ng_decrypt_tables: KeyOrigin::Absent,
            awc_key: KeyOrigin::Absent,
            snapshot_agreement: SnapshotAgreement::Unchecked,
        };
        apply_executable_awc(&mut keys, awc_payload.as_deref());
        return Ok(keys);
    }

    let ng_key_targets: Vec<&[u8; 20]> = PC_NG_KEY_HASHES.iter().collect();
    progress(ExtractionProgress {
        stage: "NG keys",
        found: 0,
        total: ng_key_targets.len(),
    });
    let (ng_key_buffers, ng_key_found) = scan_many(
        &exe,
        &ng_key_targets,
        PC_NG_KEY_PAYLOAD_LEN,
        "NG keys",
        progress,
    );

    let ng_table_targets: Vec<&[u8; 20]> = PC_NG_DECRYPT_TABLE_HASHES.iter().collect();
    progress(ExtractionProgress {
        stage: "NG decrypt tables",
        found: 0,
        total: ng_table_targets.len(),
    });
    let (ng_table_buffers, ng_table_found) = scan_many(
        &exe,
        &ng_table_targets,
        PC_NG_DECRYPT_TABLE_PAYLOAD_LEN,
        "NG decrypt tables",
        progress,
    );

    // ⚠ This is the normal path on every retail build, not a patch
    // symptom: `what_the_extractor_recovers_per_executable` records 0 of 101
    // NG key blocks and 0 of 272 decrypt tables in both editions' shipped
    // executables. Only a key snapshot can fill them.
    if lut_payload.is_none()
        || ng_key_found < ng_key_targets.len()
        || ng_table_found < ng_table_targets.len()
    {
        let mut aes_key_arr = [0u8; PC_AES_KEY_LEN];
        if aes_payload.len() == PC_AES_KEY_LEN {
            aes_key_arr.copy_from_slice(&aes_payload);
        }
        let candidates = snapshot.candidates();
        let Some((last, earlier)) = candidates.split_last() else {
            let mut keys = executable_only(
                path,
                &aes_payload,
                lut_payload.as_deref(),
                ng_key_found,
                ng_key_targets.len(),
                ng_table_found,
                ng_table_targets.len(),
            )?;
            apply_executable_awc(&mut keys, awc_payload.as_deref());
            return Ok(keys);
        };
        progress(ExtractionProgress {
            stage: "key snapshot",
            found: 0,
            total: 1,
        });
        let finish = |mut keys: Gta5Keys, source: &SnapshotSource| {
            apply_executable_awc(&mut keys, awc_payload.as_deref());
            progress(ExtractionProgress {
                stage: "key snapshot",
                found: 1,
                total: 1,
            });
            tracing::info!(
                ng_keys_found = ng_key_found,
                ng_tables_found = ng_table_found,
                snapshot = %source.describe(),
                "SHA-1 NG sweep matched 0 buckets; NG keys came from a snapshot"
            );
            keys
        };
        for source in earlier {
            match snapshot_keys(source, &aes_key_arr, lut_payload.as_deref()) {
                Ok(keys) if keys.provenance.snapshot_agreement != SnapshotAgreement::Disagrees => {
                    return Ok(finish(keys, source));
                }
                Ok(_) => tracing::warn!(
                    snapshot = %source.describe(),
                    "the snapshot describes another build; trying the next one"
                ),
                Err(e) => tracing::warn!(
                    snapshot = %source.describe(),
                    error = %e,
                    "the snapshot did not decode; trying the next one"
                ),
            }
        }
        return match snapshot_keys(last, &aes_key_arr, lut_payload.as_deref()) {
            Ok(keys) => Ok(finish(keys, last)),
            Err(e) => {
                let hint = build_extraction_failure_hint(
                    ng_key_found,
                    ng_key_targets.len(),
                    ng_table_found,
                    ng_table_targets.len(),
                );
                Err(IoError::KeyExtractionFailed(format!(
                    "found the AES key, but {ng_key_found}/{} NG key blocks and {ng_table_found}/{} NG decrypt tables matched, and {} failed to decode: {e}.\n{hint}",
                    ng_key_targets.len(),
                    ng_table_targets.len(),
                    last.describe(),
                )))
            }
        };
    }

    let mut keys = Gta5Keys::empty();

    if aes_payload.len() != PC_AES_KEY_LEN {
        return Err(IoError::KeyExtractionFailed(format!(
            "AES payload had wrong length: expected {} bytes, got {}",
            PC_AES_KEY_LEN,
            aes_payload.len()
        )));
    }
    keys.aes_key.copy_from_slice(&aes_payload);

    let lut_payload = lut_payload.expect("LUT miss diverts to the snapshot fallback");
    if lut_payload.len() != GTA5_HASH_LUT_LEN {
        return Err(IoError::KeyExtractionFailed(format!(
            "LUT payload had wrong length: expected {} bytes, got {}",
            GTA5_HASH_LUT_LEN,
            lut_payload.len()
        )));
    }
    keys.lut.copy_from_slice(&lut_payload);

    keys.ng_keys.reserve(NG_KEY_COUNT);
    for (i, slot) in ng_key_buffers.into_iter().enumerate() {
        let buf = slot.expect("scan_many returned Ok for missing NG key");
        if buf.len() != NG_KEY_LEN {
            return Err(IoError::KeyExtractionFailed(format!(
                "NG key #{} had wrong length: expected {}, got {}",
                i,
                NG_KEY_LEN,
                buf.len()
            )));
        }
        let mut arr = [0u8; NG_KEY_LEN];
        arr.copy_from_slice(&buf);
        keys.ng_keys.push(arr);
    }

    keys.ng_decrypt_tables.reserve(NG_DECRYPT_TABLE_ROUNDS);
    for round in 0..NG_DECRYPT_TABLE_ROUNDS {
        let mut group: NgDecryptTableRound = [[0u32; 256]; NG_DECRYPT_TABLE_LANES];
        for (lane, slot) in group.iter_mut().enumerate() {
            let idx = round * NG_DECRYPT_TABLE_LANES + lane;
            let buf = ng_table_buffers[idx]
                .as_ref()
                .expect("scan_many returned Ok for missing NG table");
            if buf.len() != NG_DECRYPT_TABLE_BYTES {
                return Err(IoError::KeyExtractionFailed(format!(
                    "NG decrypt table [{round}][{lane}] had wrong length: expected {}, got {}",
                    NG_DECRYPT_TABLE_BYTES,
                    buf.len()
                )));
            }
            let arr: &[u8; NG_DECRYPT_TABLE_BYTES] = buf.as_slice().try_into().unwrap();
            *slot = Gta5Keys::decode_decrypt_subtable(arr);
        }
        keys.ng_decrypt_tables.push(group);
    }

    if !keys.is_complete() {
        return Err(IoError::KeyExtractionFailed(
            "extracted key set failed completeness check".into(),
        ));
    }
    keys.provenance = KeyProvenance::all_from_executable();
    apply_executable_awc(&mut keys, awc_payload.as_deref());

    Ok(keys)
}

/// How many of each key class's fingerprints one executable matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCoverage {
    pub aes_key: (usize, usize),
    pub lut: (usize, usize),
    pub ng_keys: (usize, usize),
    pub ng_decrypt_tables: (usize, usize),
    pub awc_key: (usize, usize),
}

impl KeyCoverage {
    pub fn report(&self) -> String {
        let (a, at) = self.aes_key;
        let (l, lt) = self.lut;
        let (k, kt) = self.ng_keys;
        let (t, tt) = self.ng_decrypt_tables;
        let (w, wt) = self.awc_key;
        format!(
            "AES key {a}/{at}, hash LUT {l}/{lt}, NG keys {k}/{kt}, NG decrypt tables {t}/{tt}, \
             AWC key {w}/{wt}"
        )
    }
}

/// Run every fingerprint search over `exe_path` and count what matched,
/// consulting no snapshot.
pub fn key_coverage<P: AsRef<Path>>(exe_path: P) -> Result<KeyCoverage> {
    let path = exe_path.as_ref();
    let file = std::fs::File::open(path).map_err(|e| IoError::FileReadFailed {
        path: path.to_path_buf(),
        source: e,
    })?;
    let exe = executable_bytes(&file)?;
    let quiet = |_: ExtractionProgress| {};
    let aes = scan_one(&exe, &PC_AES_KEY_HASH, PC_AES_KEY_PAYLOAD_LEN).is_some();
    let lut = scan_one(&exe, &PC_LUT_HASH, PC_LUT_PAYLOAD_LEN).is_some();
    let awc = scan_one(&exe, &PC_AWC_KEY_HASH, AWC_KEY_BYTES).is_some();
    let key_targets: Vec<&[u8; 20]> = PC_NG_KEY_HASHES.iter().collect();
    let (_, keys_found) = scan_many(&exe, &key_targets, PC_NG_KEY_PAYLOAD_LEN, "", &quiet);
    let table_targets: Vec<&[u8; 20]> = PC_NG_DECRYPT_TABLE_HASHES.iter().collect();
    let (_, tables_found) = scan_many(
        &exe,
        &table_targets,
        PC_NG_DECRYPT_TABLE_PAYLOAD_LEN,
        "",
        &quiet,
    );
    Ok(KeyCoverage {
        aes_key: (usize::from(aes), 1),
        lut: (usize::from(lut), 1),
        ng_keys: (keys_found, key_targets.len()),
        ng_decrypt_tables: (tables_found, table_targets.len()),
        awc_key: (usize::from(awc), 1),
    })
}

/// Take the AWC key from the executable when its fingerprint matched there;
/// otherwise leave whatever the key set already holds.
fn apply_executable_awc(keys: &mut Gta5Keys, payload: Option<&[u8]>) {
    let Some(bytes) = payload.filter(|b| b.len() == AWC_KEY_BYTES) else {
        return;
    };
    for (word, chunk) in keys.awc_key.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
        *word = u32::from_le_bytes(*chunk);
    }
    keys.provenance.awc_key = KeyOrigin::Executable;
}

/// The key set an executable yields on its own: the AES key and the hash LUT,
/// with the NG and AWC classes recorded as absent rather than failed.
fn executable_only(
    path: &Path,
    aes_payload: &[u8],
    lut_payload: Option<&[u8]>,
    ng_keys_found: usize,
    ng_keys_total: usize,
    ng_tables_found: usize,
    ng_tables_total: usize,
) -> Result<Gta5Keys> {
    let lut_payload = lut_payload.ok_or_else(|| {
        IoError::KeyExtractionFailed(format!(
            "found the AES key in {} but not the hash LUT, and no key snapshot was supplied \
             to take it from",
            path.display()
        ))
    })?;
    if aes_payload.len() != PC_AES_KEY_LEN || lut_payload.len() != GTA5_HASH_LUT_LEN {
        return Err(IoError::KeyExtractionFailed(
            "AES/LUT payload length mismatch".into(),
        ));
    }
    let mut keys = Gta5Keys::empty();
    keys.aes_key.copy_from_slice(aes_payload);
    keys.lut.copy_from_slice(lut_payload);
    keys.provenance = KeyProvenance {
        aes_key: KeyOrigin::Executable,
        lut: KeyOrigin::Executable,
        ng_keys: KeyOrigin::Absent,
        ng_decrypt_tables: KeyOrigin::Absent,
        awc_key: KeyOrigin::Absent,
        snapshot_agreement: SnapshotAgreement::Unchecked,
    };
    tracing::warn!(
        ng_keys_found,
        ng_keys_total,
        ng_tables_found,
        ng_tables_total,
        "no key snapshot: {WITHOUT_A_SNAPSHOT}"
    );
    Ok(keys)
}

/// A snapshot's key set, tagged with its origin and checked against the executable's LUT.
fn snapshot_keys(
    source: &SnapshotSource,
    aes_key: &[u8; PC_AES_KEY_LEN],
    exe_lut: Option<&[u8]>,
) -> Result<Gta5Keys> {
    let mut keys = source.decode(aes_key).unwrap_or_else(|| {
        Err(IoError::KeyExtractionFailed(format!(
            "{} names no snapshot to decode",
            source.describe()
        )))
    })?;
    let origin = source.origin();
    keys.provenance = KeyProvenance {
        aes_key: KeyOrigin::Executable,
        lut: origin,
        ng_keys: origin,
        ng_decrypt_tables: origin,
        awc_key: origin,
        snapshot_agreement: SnapshotAgreement::Unchecked,
    };
    reconcile_snapshot_against_executable(&mut keys, exe_lut, source);
    Ok(keys)
}

/// Check a snapshot against the executable on the one field both supply, and
/// refuse its NG and AWC material if they disagree.
///
/// ⚠ A disagreement is a finding, not a fallback: the NG keys beside a LUT
/// that is not this build's would decrypt to noise, and noise that parses is
/// worse than a refusal. The AWC key rides in the same blob, so it goes with
/// them rather than surviving a snapshot the LUT has already disowned.
fn reconcile_snapshot_against_executable(
    keys: &mut Gta5Keys,
    exe_lut: Option<&[u8]>,
    snapshot: &SnapshotSource,
) {
    let Some(exe_lut) = exe_lut.filter(|lut| lut.len() == GTA5_HASH_LUT_LEN) else {
        return;
    };
    if exe_lut == keys.lut {
        keys.provenance.lut = KeyOrigin::Executable;
        keys.provenance.snapshot_agreement = SnapshotAgreement::Agrees;
        return;
    }
    tracing::warn!(
        snapshot = %snapshot.describe(),
        "the snapshot's hash LUT differs from this executable's; refusing its NG keys"
    );
    keys.lut.copy_from_slice(exe_lut);
    keys.ng_keys.clear();
    keys.ng_decrypt_tables.clear();
    keys.awc_key = [0; AWC_KEY_WORDS];
    keys.provenance = KeyProvenance {
        aes_key: KeyOrigin::Executable,
        lut: KeyOrigin::Executable,
        ng_keys: KeyOrigin::Absent,
        ng_decrypt_tables: KeyOrigin::Absent,
        awc_key: KeyOrigin::Absent,
        snapshot_agreement: SnapshotAgreement::Disagrees,
    };
}

#[cfg(feature = "mmap")]
fn executable_bytes(file: &std::fs::File) -> Result<memmap2::Mmap> {
    // SAFETY: the map is only read, and only while the scan that asked for it runs.
    unsafe { memmap2::Mmap::map(file) }.map_err(|e| IoError::KeyExtractionFailed(e.to_string()))
}

#[cfg(not(feature = "mmap"))]
fn executable_bytes(mut file: &std::fs::File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes)
        .map_err(|e| IoError::KeyExtractionFailed(e.to_string()))?;
    Ok(bytes)
}

fn scan_one(haystack: &[u8], target: &[u8; 20], len: usize) -> Option<Vec<u8>> {
    if haystack.len() < len {
        return None;
    }
    let last = haystack.len() - len;
    let result: Mutex<Option<Vec<u8>>> = Mutex::new(None);
    // ⚠ The scan asks this at every step and never the lock: millions of contended locks from every core panic the macOS kernel.
    let found = AtomicBool::new(false);
    let stride_count = last / ALIGN_LENGTH + 1;
    spread(0..stride_count).for_each(|i| {
        if found.load(Ordering::Relaxed) {
            return;
        }
        let off = i * ALIGN_LENGTH;
        if off > last {
            return;
        }
        let window = &haystack[off..off + len];
        let mut hasher = Sha1::new();
        hasher.update(window);
        let h = hasher.finalize();
        if h[..] == target[..] {
            found.store(true, Ordering::Relaxed);
            let mut guard = result.lock().unwrap();
            if guard.is_none() {
                *guard = Some(window.to_vec());
            }
        }
    });
    result.into_inner().unwrap()
}

fn scan_many(
    haystack: &[u8],
    targets: &[&[u8; 20]],
    len: usize,
    stage: &'static str,
    progress: &ProgressCb<'_>,
) -> (Vec<Option<Vec<u8>>>, usize) {
    if haystack.len() < len {
        return ((0..targets.len()).map(|_| None).collect(), 0);
    }
    let last = haystack.len() - len;
    let stride_count = last / ALIGN_LENGTH + 1;
    let results: Vec<Mutex<Option<Vec<u8>>>> =
        (0..targets.len()).map(|_| Mutex::new(None)).collect();
    let found_count = AtomicUsize::new(0);
    let total = targets.len();

    spread(0..stride_count).for_each(|i| {
        let off = i * ALIGN_LENGTH;
        if off > last {
            return;
        }
        let window = &haystack[off..off + len];
        let mut hasher = Sha1::new();
        hasher.update(window);
        let h = hasher.finalize();
        for (j, target) in targets.iter().enumerate() {
            if h[..] != target[..] {
                continue;
            }
            let mut slot = results[j].lock().unwrap();
            if slot.is_none() {
                *slot = Some(window.to_vec());
                let new_count = found_count.fetch_add(1, Ordering::Relaxed) + 1;
                progress(ExtractionProgress {
                    stage,
                    found: new_count,
                    total,
                });
            }
        }
    });

    let out: Vec<Option<Vec<u8>>> = results
        .into_iter()
        .map(|m| m.into_inner().unwrap())
        .collect();
    let count = found_count.load(Ordering::Relaxed);
    (out, count)
}

fn build_extraction_failure_hint(
    ng_keys_found: usize,
    ng_keys_total: usize,
    ng_tables_found: usize,
    ng_tables_total: usize,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    if ng_keys_found == 0 && ng_tables_found == 0 {
        lines.push(
            "No NG keys or NG decrypt tables matched. That is expected: no shipped \
             Legacy or Enhanced executable carries them, so the NG key set can only \
             come from a key snapshot."
                .into(),
        );
    } else if ng_keys_found == 0 {
        lines.push(format!(
            "All {ng_tables_total} NG decrypt tables matched, but none of the \
             {ng_keys_total} NG key blocks did. Partial NG-key rotation; \
             usually safe to use AES-only mode for AES-encrypted archives."
        ));
    } else if ng_tables_found == 0 {
        lines.push(format!(
            "{ng_keys_found}/{ng_keys_total} NG key blocks matched, but no NG \
             decrypt tables were found. Partial rotation; AES-only fallback \
             will still work for AES archives."
        ));
    } else {
        lines.push(format!(
            "{ng_keys_found}/{ng_keys_total} NG key blocks and \
             {ng_tables_found}/{ng_tables_total} NG decrypt tables matched. \
             Some entries were rotated by a game patch."
        ));
    }
    lines.push(
        "Workaround: re-run with the AES-only fallback to read AES-encrypted \
         archives (the majority of retail RPFs). NG-encrypted archives \
         (mostly `.ysc` scripts inside `update.rpf`) stay locked until a \
         key snapshot that decodes supplies the NG material.\n  \
         GUI:  Settings -> 'Re-extract (AES only)'\n  \
         CLI:  rpf extract-keys <gta5.exe> <out.bin> --aes-only"
            .into(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_corpus(needles: &[(&[u8], usize)], total_size: usize) -> Vec<u8> {
        let mut buf = vec![0u8; total_size];
        for (data, off) in needles {
            buf[*off..*off + data.len()].copy_from_slice(data);
        }
        buf
    }

    #[test]
    fn scan_one_finds_aligned_payload() {
        let payload = (0..32u8).collect::<Vec<u8>>();
        let mut hasher = Sha1::new();
        hasher.update(&payload);
        let hash: [u8; 20] = hasher.finalize().into();
        let buf = make_corpus(&[(&payload, 4096)], 1 << 16);
        let found = scan_one(&buf, &hash, 32).expect("hit");
        assert_eq!(found, payload);
    }

    #[test]
    fn scan_one_returns_none_when_absent() {
        let buf = vec![0u8; 1 << 14];
        let target = [0x42u8; 20];
        assert!(scan_one(&buf, &target, 32).is_none());
    }

    #[test]
    fn extraction_failure_hint_is_actionable_for_zero_ng_matches() {
        let hint = build_extraction_failure_hint(0, 101, 0, 272);
        assert!(hint.contains("Enhanced") && hint.contains("Legacy"));
        assert!(
            !hint.contains("Gen9"),
            "user-facing hint should say Enhanced"
        );
        assert!(hint.contains("--aes-only") && hint.contains("AES-only"));
    }

    #[test]
    fn extraction_failure_hint_for_partial_match_mentions_patch() {
        let hint = build_extraction_failure_hint(50, 101, 100, 272);
        assert!(hint.contains("50/101"));
        assert!(hint.contains("100/272"));
        assert!(hint.contains("--aes-only"));
    }

    #[test]
    fn scan_many_returns_partial_results_without_failing() {
        let payload_a = (0..32u8).collect::<Vec<u8>>();
        let _ = (32..64u8).collect::<Vec<u8>>();
        let mut hasher = Sha1::new();
        hasher.update(&payload_a);
        let hash_a: [u8; 20] = hasher.finalize().into();
        let hash_b = [0xFFu8; 20];
        let buf = make_corpus(&[(&payload_a, 4096)], 1 << 16);
        let targets: Vec<&[u8; 20]> = vec![&hash_a, &hash_b];
        let cb = |_: ExtractionProgress| {};
        let (results, found) = scan_many(&buf, &targets, 32, "test", &cb);
        assert_eq!(found, 1);
        assert!(results[0].is_some());
        assert!(results[1].is_none());
    }

    #[test]
    fn scanner_finds_synthetic_payloads_at_aligned_offsets() {
        let mut payloads: Vec<Vec<u8>> = Vec::new();
        for k in 0..32u8 {
            let mut p = vec![0u8; 272];
            for (i, b) in p.iter_mut().enumerate() {
                *b = k.wrapping_mul(31).wrapping_add(i as u8);
            }
            payloads.push(p);
        }

        let hashes: Vec<[u8; 20]> = payloads
            .iter()
            .map(|p| {
                let mut h = Sha1::new();
                h.update(p);
                let out: [u8; 20] = h.finalize().into();
                out
            })
            .collect();

        let mut corpus = vec![0xCCu8; 4 * 1024 * 1024];
        let mut chosen_offsets: Vec<usize> = Vec::new();
        for (k, p) in payloads.iter().enumerate() {
            let off = (k * 91_739) & !0x7;
            chosen_offsets.push(off);
            corpus[off..off + p.len()].copy_from_slice(p);
        }

        let target_refs: Vec<&[u8; 20]> = hashes.iter().collect();
        let cb = |_: ExtractionProgress| {};
        let (results, found) = scan_many(&corpus, &target_refs, 272, "synthetic", &cb);
        assert_eq!(
            found,
            payloads.len(),
            "scanner missed {} payload(s)",
            payloads.len() - found
        );
        for (i, slot) in results.iter().enumerate() {
            let bytes = slot.as_ref().expect("payload not found");
            assert_eq!(bytes, &payloads[i], "payload #{i} returned the wrong bytes");
        }
    }

    /// What the fingerprint search is actually robust to.
    ///
    /// ⚠ **Relocation, and nothing else.** A SHA-1 search can only find a
    /// payload whose hash it already holds, so it survives a patch that moves
    /// the key and not one that changes it. Measured on both retail builds by
    /// `what_a_real_executable_still_carries`: the AES key moved from
    /// 0x1E38BD8 to 0x289D6A0 between the editions and was found in both,
    /// while every NG payload is absent from both and no search finds it.
    #[test]
    fn a_relocated_payload_is_found_and_a_mutated_one_is_not() {
        let payload: Vec<u8> = (0..64u8).collect();
        let mut hasher = Sha1::new();
        hasher.update(&payload);
        let hash: [u8; 20] = hasher.finalize().into();

        for offset in [0usize, 4096, 1 << 15, (1 << 16) - 64] {
            let buf = make_corpus(&[(&payload, offset)], 1 << 16);
            assert_eq!(
                scan_one(&buf, &hash, 64).as_deref(),
                Some(&payload[..]),
                "a payload at 0x{offset:X} was not found"
            );
        }

        let mut mutated = payload.clone();
        mutated[31] ^= 0x01;
        let buf = make_corpus(&[(&mutated, 4096)], 1 << 16);
        assert!(
            scan_one(&buf, &hash, 64).is_none(),
            "one flipped bit still matched, so this is not a fingerprint search"
        );
    }

    /// ⚠ The scan strides by 8, so a payload the compiler placed on an odd
    /// boundary is invisible to it. Stated because "not found" reads as
    /// "not present" everywhere else in this module.
    #[test]
    fn a_misaligned_payload_is_invisible_to_the_scan() {
        let payload: Vec<u8> = (0..64u8).collect();
        let mut hasher = Sha1::new();
        hasher.update(&payload);
        let hash: [u8; 20] = hasher.finalize().into();
        let buf = make_corpus(&[(&payload, 4097)], 1 << 16);
        assert!(scan_one(&buf, &hash, 64).is_none());
        assert_eq!(ALIGN_LENGTH, 8);
    }

    #[test]
    fn a_snapshot_that_agrees_with_the_executable_keeps_its_ng_keys() {
        let mut keys = Gta5Keys::empty();
        keys.lut = [0x5A; GTA5_HASH_LUT_LEN];
        keys.ng_keys = vec![[1u8; NG_KEY_LEN]; NG_KEY_COUNT];
        keys.ng_decrypt_tables =
            vec![[[0u32; 256]; NG_DECRYPT_TABLE_LANES]; NG_DECRYPT_TABLE_ROUNDS];
        keys.awc_key = [0xDEAD_BEEF; AWC_KEY_WORDS];
        keys.provenance = KeyProvenance {
            aes_key: KeyOrigin::Executable,
            lut: KeyOrigin::SnapshotFile,
            ng_keys: KeyOrigin::SnapshotFile,
            ng_decrypt_tables: KeyOrigin::SnapshotFile,
            awc_key: KeyOrigin::SnapshotFile,
            snapshot_agreement: SnapshotAgreement::Unchecked,
        };

        let exe_lut = [0x5A; GTA5_HASH_LUT_LEN];
        reconcile_snapshot_against_executable(
            &mut keys,
            Some(&exe_lut),
            &SnapshotSource::File("user/magic.dat".into()),
        );

        assert!(keys.has_ng());
        // The AWC key rides in the same blob, so an agreeing snapshot keeps
        // it for the same reason it keeps the NG keys.
        assert!(keys.has_awc());
        assert_eq!(
            keys.provenance.snapshot_agreement,
            SnapshotAgreement::Agrees
        );
        assert_eq!(keys.provenance.lut, KeyOrigin::Executable);
    }

    #[test]
    fn a_snapshot_that_disagrees_has_its_ng_keys_refused_rather_than_used() {
        let mut keys = Gta5Keys::empty();
        keys.lut = [0x5A; GTA5_HASH_LUT_LEN];
        keys.ng_keys = vec![[1u8; NG_KEY_LEN]; NG_KEY_COUNT];
        keys.ng_decrypt_tables =
            vec![[[0u32; 256]; NG_DECRYPT_TABLE_LANES]; NG_DECRYPT_TABLE_ROUNDS];

        keys.awc_key = [0xDEAD_BEEF; AWC_KEY_WORDS];

        let mut exe_lut = [0x5A; GTA5_HASH_LUT_LEN];
        exe_lut[200] = 0x5B;
        reconcile_snapshot_against_executable(
            &mut keys,
            Some(&exe_lut),
            &SnapshotSource::File("user/magic.dat".into()),
        );

        assert!(
            !keys.has_ng(),
            "a snapshot for another build was used anyway"
        );
        assert!(
            !keys.has_awc(),
            "the AWC key rides in the same blob as the NG keys, so a snapshot the LUT \
             has disowned cannot keep it"
        );
        assert_eq!(keys.lut, exe_lut, "the executable's own LUT must win");
        assert_eq!(
            keys.provenance.snapshot_agreement,
            SnapshotAgreement::Disagrees
        );
        assert_eq!(keys.provenance.ng_keys, KeyOrigin::Absent);
        assert_eq!(keys.provenance.awc_key, KeyOrigin::Absent);
    }

    #[test]
    fn nothing_is_reconciled_when_the_executable_supplied_no_lut() {
        let mut keys = Gta5Keys::empty();
        keys.lut = [0x5A; GTA5_HASH_LUT_LEN];
        keys.ng_keys = vec![[1u8; NG_KEY_LEN]; NG_KEY_COUNT];
        keys.ng_decrypt_tables =
            vec![[[0u32; 256]; NG_DECRYPT_TABLE_LANES]; NG_DECRYPT_TABLE_ROUNDS];
        keys.provenance.ng_keys = KeyOrigin::SnapshotFile;

        reconcile_snapshot_against_executable(
            &mut keys,
            None,
            &SnapshotSource::File("user/magic.dat".into()),
        );
        assert!(keys.has_ng());
        assert_eq!(
            keys.provenance.snapshot_agreement,
            SnapshotAgreement::Unchecked
        );

        // A LUT of the wrong length is no evidence either.
        reconcile_snapshot_against_executable(
            &mut keys,
            Some(&[0u8; 8]),
            &SnapshotSource::File("user/magic.dat".into()),
        );
        assert_eq!(
            keys.provenance.snapshot_agreement,
            SnapshotAgreement::Unchecked
        );
    }

    #[test]
    fn only_a_cache_an_earlier_build_wrote_without_ng_keys_is_extracted_again() {
        use super::super::cache::{KeysCache, KEYS_CACHE_SCHEMA};
        let snapshot = SnapshotSource::File("user/magic.dat".into());
        let mut refused = Gta5Keys::empty();
        refused.provenance.snapshot_agreement = SnapshotAgreement::Disagrees;
        for keys in [Gta5Keys::empty(), refused] {
            let fingerprint = ExeFingerprint {
                sha1_hex: "abc".into(),
                size: 1,
            };
            let mut cache = KeysCache::build("GTA5.exe".into(), fingerprint, &keys);
            assert!(
                !snapshot.would_complete(&cache),
                "a cache this build wrote would be extracted again on every start"
            );
            cache.metadata.schema = KEYS_CACHE_SCHEMA - 1;
            assert!(snapshot.would_complete(&cache));
            assert!(!SnapshotSource::None.would_complete(&cache));
        }
    }

    #[test]
    fn a_users_snapshot_is_tried_before_the_bundled_one() {
        let file = SnapshotSource::File("user/magic.dat".into());
        assert!(SnapshotSource::None.candidates().is_empty());
        assert_eq!(file.origin(), KeyOrigin::SnapshotFile);
        #[cfg(feature = "bundled-keys")]
        {
            assert_eq!(file.candidates(), [file.clone(), SnapshotSource::Bundled]);
            assert_eq!(
                SnapshotSource::Bundled.candidates(),
                [SnapshotSource::Bundled]
            );
            assert_eq!(
                SnapshotSource::Bundled.origin(),
                KeyOrigin::EmbeddedSnapshot
            );
        }
        #[cfg(not(feature = "bundled-keys"))]
        assert_eq!(file.candidates(), [file.clone()]);
    }

    #[test]
    fn a_file_with_no_aes_key_is_refused_with_a_named_cause() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_an_exe.bin");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        let err = extract_keys_with(&path, ExtractionMode::AesOnly, &|_| {}).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not a Windows executable"), "{msg}");
        assert!(msg.contains("GTA5_Enhanced.exe"), "{msg}");
    }

    #[test]
    fn a_user_snapshot_beside_the_cache_is_found_and_nothing_else_is() {
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().to_path_buf();

        assert_eq!(SnapshotSource::discover(Some(&dir)), SnapshotSource::None);
        assert_eq!(SnapshotSource::discover(None), SnapshotSource::None);

        let path = dir.join(USER_SNAPSHOT_FILE_NAME);
        std::fs::write(&path, b"not a real snapshot").unwrap();
        assert_eq!(
            SnapshotSource::discover(Some(&dir)),
            SnapshotSource::File(path)
        );

        // A directory of that name is not a snapshot.
        std::fs::remove_file(dir.join(USER_SNAPSHOT_FILE_NAME)).unwrap();
        std::fs::create_dir(dir.join(USER_SNAPSHOT_FILE_NAME)).unwrap();
        assert_eq!(SnapshotSource::discover(Some(&dir)), SnapshotSource::None);
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fingerprint_test.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let f1 = fingerprint_executable(&path).unwrap();
        let f2 = fingerprint_executable(&path).unwrap();
        assert_eq!(f1, f2);
        assert_eq!(f1.size, 11);
        assert_eq!(f1.sha1_hex, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
    }
}
