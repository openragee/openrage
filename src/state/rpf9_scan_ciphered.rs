//! Opening a content-ciphered entry of a GTA VI archive: the request the archive explorer leaves,
//! the decipher on the load worker, and the view the deciphered body opens in.

use super::*;

/// The payload a decrypted, inflated body opens as: a PSO or RBF through the metadata reader, text
/// as text, else its bytes under the reason none reads them. A headerless body (a compiled script, a
/// texture) is kept as bytes, never decoded as Latin-1 text the way a `.bin` path otherwise would be.
fn decrypted_payload(path: &Path, bytes: Vec<u8>) -> DocPayload {
    if bytes.starts_with(b"PSIN") || bytes.starts_with(b"RBF0") {
        return DocPayload::Meta(MetaDocument::from_bytes(bytes));
    }
    if super::super::document_io::is_text(&bytes) {
        return super::super::document_io::text_payload(path, bytes);
    }
    if let Some(listing) = gta6_script_view::script_payload(&bytes) {
        return listing;
    }
    let banner = super::super::document_io::no_reader(path);
    DocPayload::Hex(HexDocument { bytes, banner })
}

/// A request to decipher a content-ciphered entry: the enciphered leaf `entry` within the archive
/// whose header is at absolute `base` in the file the synthetic body path names.
#[derive(Clone, Copy)]
pub(super) struct CipheredRequest {
    base: u64,
    entry: u32,
}

/// Requests the GTA VI archive explorer left for its content-ciphered opens, keyed by the synthetic
/// body path [`body_path`] made, taken by [`read_body`] when it opens that path. A request is a few
/// words, not the plaintext, and the decipher runs on the load worker, not the UI thread.
static CIPHERED: Mutex<BTreeMap<PathBuf, CipheredRequest>> = Mutex::new(BTreeMap::new());

/// Keep a request to decipher the leaf `entry` of the archive at `base`, for the next open of `path`.
pub(crate) fn remember_ciphered(path: &Path, base: u64, entry: u32) {
    (CIPHERED.lock())
        .unwrap_or_else(|p| p.into_inner())
        .insert(path.to_path_buf(), CipheredRequest { base, entry });
}

/// Take the decrypt request remembered for `path`, if any; a body opens from it once.
pub(super) fn take_ciphered(path: &Path) -> Option<CipheredRequest> {
    (CIPHERED.lock())
        .unwrap_or_else(|p| p.into_inner())
        .remove(path)
}

/// Decipher and inflate the content-ciphered leaf `entry_index` of the archive whose header is at
/// `base` in the file at `archive`, loading the GTA VI key snapshot for the decipher. A test helper:
/// the open path takes the stored bytes once through [`ciphered_payload`] instead.
#[path = "gta6_script_view.rs"]
mod gta6_script_view;

#[cfg(test)]
pub(crate) fn decipher_entry(
    archive: &Path,
    base: u64,
    entry_index: u32,
) -> Result<Vec<u8>, String> {
    let (keys, mut file, mount) = mount_ciphered(archive, base)?;
    let entry = (mount.archive.entries.iter())
        .find(|e| e.index == entry_index)
        .ok_or_else(|| format!("entry {entry_index} is not in this table"))?;
    mount
        .read_entry_decrypted(&mut file, entry, &keys)
        .map_err(|e| e.to_string())
}

/// Open the content-ciphered leaf `req.entry` of the archive at `req.base` in the file at `archive`,
/// deciphering its STORED bytes once: a headerless RSC8 resource opens through the step a clear
/// body takes, so it shows in the view a clear open of it shows; everything else inflates those
/// same bytes (no second decipher) and opens by their head.
pub(super) fn ciphered_payload(
    path: &Path,
    archive: &Path,
    req: CipheredRequest,
) -> Result<DocPayload, String> {
    let (keys, mut file, mount) = mount_ciphered(archive, req.base)?;
    let entry = (mount.archive.entries.iter())
        .find(|e| e.index == req.entry)
        .ok_or_else(|| format!("entry {} is not in this table", req.entry))?;
    let stored = mount
        .read_entry_deciphered_stored(&mut file, entry, &keys)
        .map_err(|e| e.to_string())?;
    if let Some(payload) = ciphered_resource(path, archive, &stored) {
        return Ok(payload);
    }
    let bytes = mount
        .inflate_deciphered_stored(&stored, entry)
        .map_err(|e| e.to_string())?;
    Ok(decrypted_payload(path, bytes))
}

/// Load the GTA VI key snapshot and mount the archive whose header is at `base` in the file at
/// `archive`, returning the keys, the open file and the mount so one open deciphers one entry.
fn mount_ciphered(
    archive: &Path,
    base: u64,
) -> Result<(std::sync::Arc<GtaViKeys>, File, Gta6Mount), String> {
    let keys = super::rpf9_table::keys()?;
    let mut file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mounted = open_at(&mut file, base, &keys).map_err(|e| e.to_string())?;
    Ok((
        keys,
        file,
        Gta6Mount {
            base,
            archive: mounted,
        },
    ))
}

/// A content-ciphered entry's deciphered STORED bytes opened as a headerless RSC8 resource of
/// `archive`, through [`body_payload`], the step a clear body takes. `None` when they are none — PSO,
/// RBF and text, whose reader's head is not a block map — so the caller inflates and opens them.
fn ciphered_resource(path: &Path, archive: &Path, stored: &[u8]) -> Option<DocPayload> {
    if !starts_stored_rpf9_resource(stored, or_io::gta6::anim::RSC8_VERSION) {
        return None;
    }
    let (find, bytes) = resource_body(stored)?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(_), ..
    }) = find.kind
    else {
        return None;
    };
    // No origin is recorded: the deciphered buffer's offset is 0, not the entry's place in the
    // archive, so the inspector would print a misleading `Offset 0x0`.
    body_payload(path, archive, find, bytes).ok()
}

/// [`read_rpf9_body`] over deciphered stored bytes at offset 0, trimming the trailing pad a lone
/// resource's file must not carry (the reader reports it as [`Rpf9ScanError::Lengthened`]).
fn resource_body(stored: &[u8]) -> Option<(Rpf9ScanFind, Vec<u8>)> {
    match read_rpf9_body(std::io::Cursor::new(stored), 0, BODY_LIMIT) {
        Ok(read) => Some(read),
        Err(Rpf9ScanError::Lengthened { end, .. }) => {
            let end = (end as usize).min(stored.len());
            read_rpf9_body(std::io::Cursor::new(&stored[..end]), 0, BODY_LIMIT).ok()
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod ciphered_open {
    use super::*;

    /// A decrypted body opens through the reader its head names, and a headerless binary body keeps
    /// its bytes under the no-reader banner rather than being decoded as Latin-1 text.
    #[test]
    fn a_decrypted_body_opens_by_its_head() {
        let path = Path::new("K:/none/archive.rpf/0x1000.bin");

        let mut psin = b"PSIN".to_vec();
        psin.extend_from_slice(&[0u8; 64]);
        assert!(
            matches!(decrypted_payload(path, psin), DocPayload::Meta(_)),
            "a decrypted PSO opens through the metadata reader"
        );

        let xml = br#"<?xml version="1.0"?><root/>"#.to_vec();
        assert!(
            matches!(
                decrypted_payload(path, xml),
                DocPayload::Meta(_) | DocPayload::Text(_)
            ),
            "decrypted XML opens as text, not as its bytes"
        );

        let mut binary = vec![0x90u8, 0x00, 0x11, 0x22];
        binary.extend_from_slice(&[0u8; 32]);
        assert!(
            matches!(decrypted_payload(path, binary), DocPayload::Hex(_)),
            "a headerless binary body keeps its bytes under the banner, not decoded as text"
        );
    }

    /// A deciphered body that is not a headerless resource is not routed through the resource reader:
    /// a PSO's stored form is zlib under a `78 DA` head, so the resource gate inflates its first bytes
    /// and finds `PSIN`, not a block map. [`ciphered_resource`] declines it and the caller inflates it.
    #[test]
    fn a_non_resource_body_is_not_routed_to_the_resource_reader() {
        use crate::state::opening_an_rpf9_archive::zlib;

        // A PSO body stored as a `78 DA` zlib block: its first inflated 16 bytes are `PSIN`, not a
        // block map, so `starts_stored_rpf9_resource` reaches its block-map check and refuses.
        let mut plain = b"PSIN".to_vec();
        plain.extend_from_slice(&[0u8; 64]);
        let mut stored = zlib(&plain);
        stored.resize(stored.len().next_multiple_of(16) + 16, 0);

        let path = Path::new("K:/none/archive.rpf/0x1000.bin");
        assert!(
            ciphered_resource(path, Path::new("K:/none/archive.rpf"), &stored).is_none(),
            "a PSO stored body must not open through the headerless resource reader"
        );
    }

    /// A deciphered stored headerless resource reaches the table-free reader and opens: its `78 DA`
    /// zlib's first bytes are a block map, so the resource gate admits it, and trailing slack past the
    /// resource makes the reader report [`Rpf9ScanError::Lengthened`], which [`resource_body`] trims.
    #[test]
    fn a_stored_headerless_resource_is_routed_and_its_trailing_slack_is_trimmed() {
        use crate::state::opening_an_rpf9_archive::{headerless_rbm, zlib};

        let stored = zlib(&headerless_rbm(0x5000_00E0));
        assert!(
            starts_stored_rpf9_resource(&stored, or_io::gta6::anim::RSC8_VERSION),
            "a stored headerless resource is admitted by the resource gate"
        );
        // Read without slack: the reader finds the resource at offset 0.
        let (exact, _) = resource_body(&stored).expect("the exact stored bytes hold a resource");

        // The same bytes with trailing slack: the reader reports Lengthened, and the trim recovers the
        // same resource at the same offset and kind.
        let mut padded = stored.clone();
        padded.resize(padded.len() + 0x80, 0);
        let (trimmed, _) =
            resource_body(&padded).expect("the Lengthened trim recovers the resource");
        assert_eq!(
            trimmed.offset, exact.offset,
            "the trimmed read is the same body"
        );
        assert_eq!(
            trimmed.kind, exact.kind,
            "the trimmed read is the same kind"
        );
    }

    /// The same stored headerless bodies open alike through the clear door and the deciphered door,
    /// each as a read and not its bytes: headerless metadata, and a clip root, which the deciphered
    /// door once left as bytes because it routed only a texture dictionary and headerless metadata.
    #[test]
    fn a_stored_headerless_body_opens_alike_through_both_doors() {
        use crate::state::opening_an_rpf9_archive::{
            animation_set, headerless_rbm, rpf9_head, written, zlib,
        };

        let mut bytes = rpf9_head(1, 0x1A0);
        let mut offsets = Vec::new();
        for body in [zlib(&headerless_rbm(0x5000_00E0)), zlib(&animation_set().0)] {
            // A headerless resource follows a zero 16-byte slot at the start of a 4 KiB page.
            bytes.resize(bytes.len().next_multiple_of(0x1000) + 16, 0);
            offsets.push(bytes.len() as u64);
            bytes.extend_from_slice(&body);
        }
        let dir = tempfile::tempdir().expect("a scratch directory");
        let disk = written(dir.path(), &bytes);
        let ends = (offsets.iter().skip(1).copied()).chain([bytes.len() as u64]);
        for (start, end) in offsets.iter().copied().zip(ends) {
            let clear = read_body(&body_path(&disk, start, View::Headerless), &disk, start)
                .unwrap_or_else(|e| panic!("the clear door opens {start:#X}: {e}"));
            // The stored bytes run on to the next body, as a deciphered entry's pad does.
            let stored = &bytes[start as usize..end as usize];
            let deciphered =
                ciphered_resource(&body_path(&disk, start, View::Bytes), &disk, stored)
                    .unwrap_or_else(|| panic!("the deciphered door opens {start:#X}"));
            assert!(
                !matches!(clear, DocPayload::Hex(_)),
                "the clear door reads the body at {start:#X}"
            );
            assert_eq!(
                std::mem::discriminant(&deciphered),
                std::mem::discriminant(&clear),
                "both doors open the body at {start:#X} as the same kind of payload"
            );
        }
    }
}

/// Opening a flagged GTA VI texture entry of a real build yields a texture dictionary, not hex: its
/// deciphered STORED bytes go through the same table-free resource reader a clear resource takes.
/// Install-gated — it needs the key snapshot and the build, and skips cleanly without them.
#[cfg(test)]
mod ciphered_texture_open {
    use super::*;
    use or_io::container::rpf::{Gta6Mount, GtaViKeys};

    const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
    /// A flagged leaf larger than this is left alone: a big body would decipher and inflate for seconds.
    const SAMPLE_CAP: u64 = 0x80000;
    /// How deep the walk mounts nested archives, and how many flagged leaves it opens, before it skips.
    const MAX_DEPTH: usize = 4;
    const MAX_TRIES: usize = 250;

    fn build_root() -> Option<PathBuf> {
        std::env::var_os(BUILD_ENV)
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
    }

    #[test]
    fn a_flagged_texture_entry_opens_as_a_texture() {
        let keys_set = std::env::var_os("OPENRAGE_GTA6_KEYS")
            .map(PathBuf::from)
            .is_some_and(|p| p.exists());
        let Some(keys) = build_root()
            .filter(|_| keys_set)
            .and_then(|_| GtaViKeys::for_install(or_title::config::config_dir().as_deref()).ok())
            .flatten()
        else {
            or_test_support::skip_install(
                "gta6 ciphered texture open",
                format_args!("set OPENRAGE_GTA6_KEYS (with content keys) and {BUILD_ENV}"),
            );
            return;
        };
        let root = build_root().expect("the build root was present above");
        let archives = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"));
        let mut tries = 0usize;
        for disk in archives {
            if tries >= MAX_TRIES {
                break;
            }
            let Ok(mut file) = File::open(&disk) else {
                continue;
            };
            let Ok(root_mount) = Gta6Mount::open_root(&mut file, &keys) else {
                continue;
            };
            let mut stack = vec![(root_mount, 0usize)];
            while let Some((mount, depth)) = stack.pop() {
                for entry in &mount.archive.entries {
                    if tries >= MAX_TRIES {
                        break;
                    }
                    if mount
                        .entry_begins_archive(&mut file, entry)
                        .unwrap_or(false)
                    {
                        if depth + 1 < MAX_DEPTH {
                            if let Ok(child) = mount.open_nested(&mut file, entry, &keys) {
                                stack.push((child, depth + 1));
                            }
                        }
                        continue;
                    }
                    let small = entry.cipher.is_some()
                        && entry
                            .real
                            .or(entry.stored)
                            .is_some_and(|s| s > 0 && s <= SAMPLE_CAP);
                    let Some(start) = mount.absolute_start(entry).filter(|_| small) else {
                        continue;
                    };
                    tries += 1;
                    let path = body_path(&disk, start, View::Bytes);
                    remember_ciphered(&path, mount.base, entry.index);
                    let Ok(DocPayload::Resource(or_io::Asset::TextureDict(dict))) =
                        read_body(&path, &disk, start)
                    else {
                        continue;
                    };
                    assert!(
                        dict.entries
                            .iter()
                            .flatten()
                            .any(|t| t.width > 0 && t.height > 0),
                        "a ciphered texture dictionary opened with no sane texture dimensions"
                    );
                    return;
                }
            }
        }
        or_test_support::skip_install(
            "gta6 ciphered texture open",
            format_args!("no flagged texture entry opened within {MAX_TRIES} tries"),
        );
    }
}
