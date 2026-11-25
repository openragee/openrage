//! Install check of a content-ciphered texture dictionary opened through its bounded table entry.

use super::*;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const SAMPLE_CAP: u64 = 0x80000;
const MAX_DEPTH: usize = 4;
const MAX_TRIES: usize = 250;

#[test]
fn a_flagged_texture_entry_opens_as_a_texture() {
    let root = std::env::var_os(BUILD_ENV)
        .map(PathBuf::from)
        .filter(|path| path.is_dir());
    let have_keys = std::env::var_os("OPENRAGE_GTA6_KEYS")
        .map(PathBuf::from)
        .is_some_and(|path| path.is_file());
    let Some(root) = root.filter(|_| have_keys) else {
        or_test_support::skip_install(
            "gta6 ciphered texture open",
            format_args!("set OPENRAGE_GTA6_KEYS (with content keys) and {BUILD_ENV}"),
        );
        return;
    };
    let keys = rpf9_table::keys().expect("the configured key snapshot opens");
    if !keys.has_content_keys() {
        or_test_support::skip_install(
            "gta6 ciphered texture open",
            format_args!("the key snapshot carries no content keys"),
        );
        return;
    }
    let found = or_title::gta6_archives::paths(
        &root,
        or_title::gta6_archives::Limits {
            folders: 8192,
            archives: 8192,
        },
        or_io::ErrorMode::Strict,
        &|| false,
    )
    .expect("the install's archive paths are readable");
    assert!(!found.truncated, "the install exceeds the discovery budget");
    let mut tries = 0;
    for path in found.paths {
        if tries >= MAX_TRIES {
            break;
        }
        let Some(table) =
            Rpf9TableDocument::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        else {
            continue;
        };
        let mut pending = vec![(0, 0)];
        while let Some((base, depth)) = pending.pop() {
            let entries = table.tree().nodes[&base].mount.archive.entries.clone();
            for entry in entries {
                if tries >= MAX_TRIES {
                    break;
                }
                let kind = table.tree().entry_type(base, entry.index).0;
                if kind == Gta6EntryType::Archive {
                    if depth + 1 < MAX_DEPTH {
                        let child = table.expand_table(base, entry.index).unwrap_or_else(|e| {
                            panic!("{}:{base:#x}/{}: {e}", path.display(), entry.index)
                        });
                        pending.push((child, depth + 1));
                    }
                    continue;
                }
                if entry.cipher.is_none() {
                    continue;
                }
                let source = table.entry(base, entry.index).unwrap();
                if source.decoded_len() == 0 || source.decoded_len() > SAMPLE_CAP {
                    continue;
                }
                tries += 1;
                if kind != Gta6EntryType::TextureDictionary {
                    continue;
                }
                let payload = source.payload_limited(SAMPLE_CAP).unwrap_or_else(|e| {
                    panic!("{}:{base:#x}/{}: {e}", path.display(), entry.index)
                });
                let DocPayload::Resource(or_io::Asset::TextureDict(dictionary)) = &payload else {
                    panic!("a content-ciphered texture dictionary must open in its preview")
                };
                assert!(dictionary
                    .entries
                    .iter()
                    .flatten()
                    .any(|texture| texture.width > 0 && texture.height > 0));
                let bytes = source.bytes_limited(SAMPLE_CAP).unwrap();
                let mut extracted = Vec::new();
                source.extract(&mut extracted).unwrap();
                assert_eq!(extracted, bytes);
                assert!(!source.path.exists());
                return;
            }
        }
    }
    or_test_support::skip_install(
        "gta6 ciphered texture open",
        format_args!("no flagged texture entry within {MAX_TRIES} bounded candidates"),
    );
}
