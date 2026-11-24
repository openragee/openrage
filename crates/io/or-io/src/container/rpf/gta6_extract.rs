//! The GTA VI key snapshot (`ORGTAVIK`) read out of the user's own executable: the key registry's
//! index, content and string keys, each cipher's shared tables and the two fixed IVs, at the
//! addresses a known build states and checked by their markers.
//!
//! ⚠ Only an executable whose SHA-256 names a build listed here is read; any other is refused, never
//! scanned. The addresses, markers and layout are committable; the bytes they point at are not.

use byteorder::{ByteOrder, LE};
use sha2::{Digest, Sha256};

use super::rpf9_toc::STRING_KEY_ID;
use crate::error::{IoError, Result};

/// A build this extractor knows: its executable's SHA-256 and where its key material lies.
struct Build {
    name: &'static str,
    sha256: &'static str,
    /// The key registry: `records` entries of `{u64 registry id, u64 blob address, u64 blob size}`.
    registry: u64,
    records: usize,
    /// Each cipher's shared tables: 16 input byte tables, then 16 round word tables right after.
    index_tables: u64,
    content_tables: u64,
    string_tables: u64,
    bootstrap_iv: u64,
    string_iv: u64,
}

const BUILDS: &[Build] = &[Build {
    name: "Xbox Series 1.0.2.1",
    sha256: "e95020b2fa4a721db3df59fdecf75621730ac559d8835252f6c3b658d5a08993",
    registry: 0x1_466E_5900,
    records: 24,
    index_tables: 0x1_4603_6F30,
    content_tables: 0x1_4604_0F30,
    string_tables: 0x1_460C_ABE0,
    bootstrap_iv: 0x1_462F_D470,
    string_iv: 0x1_4659_E620,
}];

const SNAPSHOT_MAGIC: &[u8; 8] = b"ORGTAVIK";
const SNAPSHOT_VERSION: u32 = 1;
/// The section tags `GtaViKeys::parse` reads: `'BC'` index (CBC), `'CR'` content (CTR), `'ST'` strings.
const MODE_INDEX: u32 = 0x4342;
const MODE_CONTENT: u32 = 0x5243;
const MODE_STRING: u32 = 0x5453;
/// The markers the three white-box key representations open with.
const INDEX_MAGIC: [u8; 4] = [0x8e, 0x9b, 0x3b, 0x4e];
const CONTENT_MAGIC: [u8; 4] = [0x5d, 0xce, 0x18, 0x27];
const STRING_MAGIC: [u8; 4] = [0x50, 0x4f, 0x86, 0x5e];
/// A key blob: the marker, 60 round-key words, sixteen 256-byte output maps.
const BLOB_LEN: usize = 0x10F4;
const INPUT_TABLES_LEN: usize = 0x1000;
const ROUND_TABLES_LEN: usize = 0x4000;
const RECORD_LEN: usize = 24;
const IV_LEN: usize = 16;
const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;
/// The decrypt-direction registry id of the string key; the encrypt one shares its key id.
const STRING_REGISTRY_ID: u64 = ((STRING_KEY_ID as u64) << 32) | 0x0200_0000;

/// The plaintext `ORGTAVIK` snapshot of the GTA VI executable `exe`; refused unless `exe` is a known build.
pub fn extract_snapshot(exe: &[u8]) -> Result<Vec<u8>> {
    let sha256: [u8; 32] = Sha256::digest(exe).into();
    let hex: String = sha256.iter().map(|b| format!("{b:02x}")).collect();
    let build = BUILDS.iter().find(|b| b.sha256 == hex).ok_or_else(|| {
        let known: Vec<&str> = BUILDS.iter().map(|b| b.name).collect();
        bad(format!(
            "its SHA-256 is {hex}, which is not a build this extractor knows ({})",
            known.join(", ")
        ))
    })?;
    tracing::info!(build = build.name, "reading the GTA VI key snapshot");
    snapshot_from(exe, build, &sha256)
}

fn snapshot_from(exe: &[u8], build: &Build, sha256: &[u8; 32]) -> Result<Vec<u8>> {
    let mut index = Vec::new();
    let mut content = Vec::new();
    let mut string = None;
    for i in 0..build.records {
        let at = build.registry + (i * RECORD_LEN) as u64;
        let record = read_va(exe, at, RECORD_LEN)?;
        let id = LE::read_u64(record);
        let va = LE::read_u64(&record[8..]);
        let size = LE::read_u64(&record[16..]);
        if size < 4 {
            continue;
        }
        let magic = read_va(exe, va, 4)?;
        let wanted = magic == INDEX_MAGIC
            || magic == CONTENT_MAGIC
            || (magic == STRING_MAGIC && id == STRING_REGISTRY_ID);
        if !wanted {
            continue;
        }
        if size != BLOB_LEN as u64 {
            return Err(bad(format!(
                "registry record {i} ({id:#018X}) states {size:#X} bytes, not {BLOB_LEN:#X}"
            )));
        }
        let blob = read_va(exe, va, BLOB_LEN)?;
        if magic == INDEX_MAGIC {
            index.push((id, blob));
        } else if magic == CONTENT_MAGIC {
            content.push((id, blob));
        } else {
            string = Some(blob);
        }
    }
    if !index.iter().any(|(id, _)| *id == BOOTSTRAP_REGISTRY_ID) {
        return Err(bad("the key registry holds no bootstrap key"));
    }
    if content.is_empty() {
        return Err(bad("the key registry holds no content key"));
    }
    let string = string.ok_or_else(|| bad("the key registry holds no string key"))?;

    let mut out = Vec::new();
    out.extend_from_slice(SNAPSHOT_MAGIC);
    out.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
    out.extend_from_slice(&MODE_INDEX.to_le_bytes());
    out.extend_from_slice(sha256);
    out.extend_from_slice(read_va(exe, build.bootstrap_iv, IV_LEN)?);
    put_tables(&mut out, exe, build.index_tables)?;
    put_blobs(&mut out, &index);
    out.extend_from_slice(&MODE_CONTENT.to_le_bytes());
    put_tables(&mut out, exe, build.content_tables)?;
    put_blobs(&mut out, &content);
    out.extend_from_slice(&MODE_STRING.to_le_bytes());
    put_tables(&mut out, exe, build.string_tables)?;
    out.extend_from_slice(read_va(exe, build.string_iv, IV_LEN)?);
    out.extend_from_slice(&(BLOB_LEN as u32).to_le_bytes());
    out.extend_from_slice(string);
    Ok(out)
}

fn put_tables(out: &mut Vec<u8>, exe: &[u8], va: u64) -> Result<()> {
    out.extend_from_slice(&(INPUT_TABLES_LEN as u32).to_le_bytes());
    out.extend_from_slice(read_va(exe, va, INPUT_TABLES_LEN)?);
    out.extend_from_slice(&(ROUND_TABLES_LEN as u32).to_le_bytes());
    out.extend_from_slice(read_va(
        exe,
        va + INPUT_TABLES_LEN as u64,
        ROUND_TABLES_LEN,
    )?);
    Ok(())
}

fn put_blobs(out: &mut Vec<u8>, blobs: &[(u64, &[u8])]) {
    out.extend_from_slice(&(blobs.len() as u32).to_le_bytes());
    for (id, blob) in blobs {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(blob);
    }
}

/// The `len` bytes at virtual address `va` of the PE image `exe`, mapped through its section table.
fn read_va(exe: &[u8], va: u64, len: usize) -> Result<&[u8]> {
    pe_map(exe, va, len).ok_or_else(|| bad(format!("no section holds {len:#X} bytes at {va:#X}")))
}

fn pe_map(exe: &[u8], va: u64, len: usize) -> Option<&[u8]> {
    let pe = u32::from_le_bytes(le(exe, 60)?) as usize;
    let sections = u16::from_le_bytes(le(exe, pe.checked_add(6)?)?) as usize;
    let optional = u16::from_le_bytes(le(exe, pe.checked_add(20)?)?) as usize;
    let base = u64::from_le_bytes(le(exe, pe.checked_add(48)?)?);
    let rva = va.checked_sub(base)?;
    let end = rva.checked_add(len as u64)?;
    let table = pe.checked_add(24)?.checked_add(optional)?;
    for i in 0..sections {
        let sh = table.checked_add(i.checked_mul(40)?)?;
        let start = u64::from(u32::from_le_bytes(le(exe, sh.checked_add(12)?)?));
        let raw_size = u64::from(u32::from_le_bytes(le(exe, sh.checked_add(16)?)?));
        let raw_ptr = u64::from(u32::from_le_bytes(le(exe, sh.checked_add(20)?)?));
        if start <= rva && end <= start.checked_add(raw_size)? {
            let begin = usize::try_from(raw_ptr.checked_add(rva - start)?).ok()?;
            return exe.get(begin..begin.checked_add(len)?);
        }
    }
    None
}

fn le<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn bad(why: impl Into<String>) -> IoError {
    IoError::Invalid {
        what: "a GTA VI executable".into(),
        why: why.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::rpf::gta6_keygate::{decode_masked, encode_masked, gate_key, is_masked};
    use crate::container::rpf::GtaViKeys;

    const BASE: u64 = 0x1_4000_0000;
    const PE: usize = 0x80;
    const OPTIONAL: usize = 0xF0;
    /// Section A holds the index tables at the build's own address, which the mask gate keys on.
    const A_RAW: usize = 0x400;
    const A_LEN: usize = INPUT_TABLES_LEN + ROUND_TABLES_LEN;
    /// Section B holds the registry, the IVs, the other tables and the blobs.
    const B_RVA: u64 = 0x10000;
    const B_RAW: usize = A_RAW + A_LEN;
    const B_LEN: usize = 0x10600;
    const REGISTRY: usize = 0;
    const BOOT_IV: usize = 0x100;
    const STR_IV: usize = 0x110;
    const CONTENT_TABLES: usize = 0x1000;
    const STRING_TABLES: usize = 0x6000;
    const BLOBS: usize = 0xB000;
    const SLOT: usize = 0x1100;
    const INDEX_ID: u64 = 0x1000_0015_0200_0000;
    const CONTENT_KEY_ID: u32 = 0x1000_0029;
    const CONTENT_ID: u64 = ((CONTENT_KEY_ID as u64) << 32) | 0x0200_0000;

    /// A synthetic two-section PE64 and the build that describes it; `seed` varies every filled byte.
    /// The registry: two index keys, one record of a foreign representation, one content key and the
    /// string key.
    fn image(seed: u8) -> (Vec<u8>, Build) {
        let mut exe = vec![0u8; B_RAW + B_LEN];
        exe[60..64].copy_from_slice(&(PE as u32).to_le_bytes());
        exe[PE..PE + 4].copy_from_slice(b"PE\0\0");
        exe[PE + 6..PE + 8].copy_from_slice(&2u16.to_le_bytes());
        exe[PE + 20..PE + 22].copy_from_slice(&(OPTIONAL as u16).to_le_bytes());
        exe[PE + 48..PE + 56].copy_from_slice(&BASE.to_le_bytes());
        let index_tables = BUILDS[0].index_tables;
        let sections = [(index_tables - BASE, A_LEN, A_RAW), (B_RVA, B_LEN, B_RAW)];
        for (i, (rva, len, raw)) in sections.into_iter().enumerate() {
            let sh = PE + 24 + OPTIONAL + i * 40;
            exe[sh + 12..sh + 16].copy_from_slice(&(rva as u32).to_le_bytes());
            exe[sh + 16..sh + 20].copy_from_slice(&(len as u32).to_le_bytes());
            exe[sh + 20..sh + 24].copy_from_slice(&(raw as u32).to_le_bytes());
        }
        for (i, b) in exe[A_RAW..].iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(31).wrapping_add(seed);
        }
        let records: [(u64, [u8; 4], usize); 5] = [
            (BOOTSTRAP_REGISTRY_ID, INDEX_MAGIC, BLOB_LEN),
            (INDEX_ID, INDEX_MAGIC, BLOB_LEN),
            (0xFFFF_FFFE_0000_0000, [1, 2, 3, 4], 0x20),
            (CONTENT_ID, CONTENT_MAGIC, BLOB_LEN),
            (STRING_REGISTRY_ID, STRING_MAGIC, BLOB_LEN),
        ];
        for (k, (id, magic, size)) in records.into_iter().enumerate() {
            let blob = B_RAW + BLOBS + k * SLOT;
            exe[blob..blob + 4].copy_from_slice(&magic);
            let r = B_RAW + REGISTRY + k * RECORD_LEN;
            exe[r..r + 8].copy_from_slice(&id.to_le_bytes());
            let va = BASE + B_RVA + (BLOBS + k * SLOT) as u64;
            exe[r + 8..r + 16].copy_from_slice(&va.to_le_bytes());
            exe[r + 16..r + 24].copy_from_slice(&(size as u64).to_le_bytes());
        }
        let b = |off: usize| BASE + B_RVA + off as u64;
        let build = Build {
            name: "synthetic",
            sha256: "",
            registry: b(REGISTRY),
            records: records.len(),
            index_tables,
            content_tables: b(CONTENT_TABLES),
            string_tables: b(STRING_TABLES),
            bootstrap_iv: b(BOOT_IV),
            string_iv: b(STR_IV),
        };
        (exe, build)
    }

    #[test]
    fn a_virtual_address_maps_through_its_section_to_the_file_bytes() {
        let (exe, build) = image(1);
        assert_eq!(
            read_va(&exe, build.bootstrap_iv, IV_LEN).expect("mapped"),
            &exe[B_RAW + BOOT_IV..B_RAW + BOOT_IV + IV_LEN]
        );
        assert_eq!(
            read_va(&exe, build.index_tables, A_LEN).expect("mapped"),
            &exe[A_RAW..A_RAW + A_LEN]
        );
    }

    #[test]
    fn an_address_no_section_holds_is_refused() {
        let (exe, build) = image(1);
        assert!(read_va(&exe, 0, 4).is_err(), "below the image base");
        assert!(read_va(&exe, BASE + 0x8000, 4).is_err(), "between sections");
        let last = BASE + B_RVA + B_LEN as u64 - 8;
        assert!(read_va(&exe, last, 16).is_err(), "past the section end");
        assert!(
            read_va(&exe[..0x100], build.bootstrap_iv, 4).is_err(),
            "truncated"
        );
        assert!(
            read_va(&[0u8; 8], build.bootstrap_iv, 4).is_err(),
            "not a PE"
        );
    }

    #[test]
    fn the_registry_walk_takes_each_key_kind_in_order_and_the_snapshot_parses() {
        let (exe, build) = image(1);
        let sha = [7u8; 32];
        let plain = snapshot_from(&exe, &build, &sha).expect("the synthetic build extracts");
        let keys = GtaViKeys::parse(&plain).expect("the extracted snapshot parses");
        assert_eq!(keys.exe_sha256(), &sha);
        assert!(keys.has_content_keys() && keys.has_string_keys());
        // The index section: two keys, in registry order; the foreign record is skipped.
        let count_at = 8 + 4 + 4 + 32 + IV_LEN + 4 + INPUT_TABLES_LEN + 4 + ROUND_TABLES_LEN;
        assert_eq!(LE::read_u32(&plain[count_at..]), 2);
        let first = count_at + 4;
        let second = first + 8 + 4 + BLOB_LEN;
        assert_eq!(LE::read_u64(&plain[first..]), BOOTSTRAP_REGISTRY_ID);
        assert_eq!(LE::read_u64(&plain[second..]), INDEX_ID);
        // The content key is filed under the id its key id selects.
        let mut data = [0u8; 32];
        assert!(keys
            .content_decrypt(CONTENT_KEY_ID, &[0; 16], &mut data)
            .is_ok());
        assert!(keys
            .content_decrypt(CONTENT_KEY_ID + 1, &[0; 16], &mut data)
            .is_err());
    }

    #[test]
    fn a_malformed_registry_is_refused() {
        let record = |k: usize| B_RAW + REGISTRY + k * RECORD_LEN;
        let (mut exe, build) = image(1);
        exe[record(1) + 16..record(1) + 24].copy_from_slice(&0x10F0u64.to_le_bytes());
        assert!(
            snapshot_from(&exe, &build, &[0; 32]).is_err(),
            "a key blob of the wrong size"
        );

        let (mut exe, build) = image(1);
        exe[record(0)..record(0) + 8].copy_from_slice(&INDEX_ID.to_le_bytes());
        assert!(
            snapshot_from(&exe, &build, &[0; 32]).is_err(),
            "no bootstrap key"
        );

        let (mut exe, build) = image(1);
        exe[record(4)..record(4) + 8].copy_from_slice(&(STRING_REGISTRY_ID + 1).to_le_bytes());
        assert!(
            snapshot_from(&exe, &build, &[0; 32]).is_err(),
            "no string key"
        );

        let (mut exe, build) = image(1);
        exe[record(3) + 8..record(3) + 16].copy_from_slice(&0u64.to_le_bytes());
        assert!(
            snapshot_from(&exe, &build, &[0; 32]).is_err(),
            "a blob nowhere"
        );
    }

    #[test]
    fn an_executable_no_known_build_matches_is_refused() {
        let (exe, _) = image(1);
        let err = extract_snapshot(&exe).expect_err("an unknown build is refused");
        assert!(
            err.to_string().contains("not a build this extractor knows"),
            "{err}"
        );
    }

    #[test]
    fn a_masked_snapshot_opens_only_with_the_executable_that_keyed_it() {
        let (exe, build) = image(1);
        let (other, _) = image(2);
        assert_ne!(gate_key(&exe), gate_key(&other));
        let plain = snapshot_from(&exe, &build, &[0; 32]).expect("extracts");
        let masked = encode_masked(&plain, &exe);
        assert!(is_masked(&masked));
        assert!(
            decode_masked(&masked, &other).is_err(),
            "another executable's gate key"
        );
        assert_eq!(
            decode_masked(&masked, &exe).expect("its own executable"),
            plain
        );
    }

    /// Install-gated: the real executable extracts, the snapshot parses and deciphers a build
    /// archive's table of contents, and it equals a staged snapshot of the same executable.
    #[test]
    fn the_extracted_snapshot_deciphers_a_build_archive() {
        use std::io::Read;
        use std::path::PathBuf;

        use crate::container::rpf::{Rpf9Toc, GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV};

        const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
        const EXE_NAME: &str = "game_scarlett_master.exe";
        let what = "GTA VI key extraction from the executable";
        let env = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let build = env(BUILD_ENV);
        let exe = env(GTA6_EXE_ENV).or_else(|| {
            build
                .as_ref()
                .map(|b| b.join(EXE_NAME))
                .filter(|p| p.is_file())
        });
        let (Some(exe), Some(build)) = (exe, build) else {
            or_test_support::skip_install(
                what,
                format_args!("set {BUILD_ENV} (and {GTA6_EXE_ENV} when {EXE_NAME} is elsewhere)"),
            );
            return;
        };
        let exe = std::fs::read(&exe).expect("read the executable");
        let plain = extract_snapshot(&exe).expect("the executable is a known build");
        let keys = GtaViKeys::parse(&plain).expect("the extracted snapshot parses");
        assert!(keys.has_content_keys() && keys.has_string_keys());

        let mut archives: Vec<PathBuf> = std::fs::read_dir(&build)
            .expect("list the build root")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rpf"))
            .collect();
        archives.sort();
        let archive = archives.first().expect("the build root holds an archive");
        let mut file = std::fs::File::open(archive).expect("open the archive");
        let mut head = [0u8; 64];
        file.read_exact(&mut head).expect("read the header");
        let toc_end = LE::read_u32(&head[4..]) as usize;
        let mut toc = head.to_vec();
        toc.resize(toc_end, 0);
        file.read_exact(&mut toc[64..])
            .expect("read the table of contents");
        let toc = Rpf9Toc::decrypt(&toc, &keys).expect("the extracted keys decipher the TOC");
        assert!(!toc.groups.is_empty(), "the deciphered index has groups");

        if let Some(staged) = env(GTA6_SNAPSHOT_ENV) {
            let staged = std::fs::read(staged).expect("read the staged snapshot");
            let staged = if is_masked(&staged) {
                decode_masked(&staged, &exe).expect("the staged snapshot unmasks")
            } else {
                staged
            };
            if staged.get(16..48) == plain.get(16..48) {
                assert_eq!(
                    staged, plain,
                    "the extraction reproduces the staged snapshot"
                );
            }
        }
    }
}
