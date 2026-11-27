//! RDR2's RPF8, read and written against the archives a retail install ships.
//!
//! ⚠ Install-gated: `OPENRAGE_GAME_ROOT` names an RDR2 install. Only an archive whose table is in
//! the clear (header tag `0xFF`) opens without the title's TFIT keys, which no provider here holds;
//! every other archive must be refused by name, not read as noise.

use std::path::{Path, PathBuf};

use or_io::container::rpf::{
    BodyForm, Compressor, RpfArchive, RpfEncryption, RpfEntry, RpfVersion,
};
use or_io::error::IoError;
use or_io::hash::string_hash;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

/// An archive of the install that is RPF8, with the tag its header carries.
fn rpf8_archives(root: &Path) -> Vec<(PathBuf, u16)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("rpf"))
        {
            continue;
        }
        let mut head = [0u8; 16];
        let read = std::fs::File::open(&path)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head));
        if read.is_ok()
            && RpfVersion::detect([head[0], head[1], head[2], head[3]]) == Some(RpfVersion::Rpf8)
        {
            found.push((path, u16::from_le_bytes([head[12], head[13]])));
        }
    }
    found.sort();
    found
}

fn installs(what: &str) -> Option<Vec<Vec<(PathBuf, u16)>>> {
    let roots = game_roots();
    let found: Vec<_> = roots
        .iter()
        .map(|r| rpf8_archives(r))
        .filter(|a| !a.is_empty())
        .collect();
    if found.is_empty() {
        skip_install(what, format_args!("no RPF8 archive under {GAME_ROOT_ENV}"));
        return None;
    }
    Some(found)
}

fn open(path: &Path) -> Result<RpfArchive, IoError> {
    RpfArchive::open(path)
}

#[test]
fn an_archive_with_a_plain_table_opens_and_every_other_is_refused_by_name() {
    let Some(installs) = installs("rpf8 tables") else {
        return;
    };
    let (mut plain, mut refused) = (0, 0);
    for (path, tag) in installs.into_iter().flatten() {
        match open(&path) {
            Ok(archive) => {
                assert_eq!(tag, 0xFF, "{} opened with tag {tag:#x}", path.display());
                assert_eq!(archive.version, RpfVersion::Rpf8);
                assert_eq!(archive.header.encryption, RpfEncryption::None);
                // RPF8 records its build platform in the header; a retail PC install reads `y`.
                let letter = archive
                    .platform_letter()
                    .unwrap_or_else(|| panic!("{} states no platform", path.display()));
                assert!(
                    "ywxdzcopasnv".contains(letter),
                    "{} states a non-RAGE platform letter {letter:?}",
                    path.display()
                );
                plain += 1;
            }
            Err(IoError::RpfEncryptionUnsupported { key_id }) => {
                assert_eq!(key_id, u32::from(tag), "{}", path.display());
                refused += 1;
            }
            Err(other) => panic!(
                "{} was neither read nor refused by key: {other}",
                path.display()
            ),
        }
    }
    eprintln!("rpf8: {plain} plain table(s) read, {refused} refused for want of a TFIT key");
}

/// Each file of every plain archive, named by its hash, with the size its row states.
#[test]
fn every_body_of_a_plain_archive_is_the_size_its_row_states() {
    let Some(installs) = installs("rpf8 bodies") else {
        return;
    };
    for (path, tag) in installs
        .into_iter()
        .flatten()
        .filter(|(_, tag)| *tag == 0xFF)
    {
        let archive = open(&path).expect("a plain table opens");
        assert!(archive.file_count() > 0, "{}", path.display());
        for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
            let i = node.entry_index();
            match &archive.entries[i as usize] {
                RpfEntry::Binary(b) if b.compressor == Compressor::Oodle => {
                    let bytes = archive
                        .read_entry(i)
                        .unwrap_or_else(|e| panic!("{}: {e}", node.path()));
                    assert_eq!(bytes.len(), b.uncompressed_size as usize, "{}", node.path());
                    let stored = archive.read_form(i, BodyForm::AsStored).expect("stored");
                    assert_eq!(stored.len(), b.consumed_size as usize);
                }
                RpfEntry::Binary(b) if !b.encrypted => {
                    let bytes = archive
                        .read_entry(i)
                        .unwrap_or_else(|e| panic!("{}: {e}", node.path()));
                    assert_eq!(bytes.len(), b.uncompressed_size as usize, "{}", node.path());
                }
                _ => {}
            }
        }
        eprintln!(
            "rpf8: {} ({tag:#x}) holds {} file(s)",
            path.display(),
            archive.file_count()
        );
    }
}

/// A retail archive spells its files from the names blob at its end: each hash is the one the engine's
/// string hash gives the name, which is what proves the hash and the blob are read as they are written.
#[test]
fn a_names_blob_spells_exactly_the_hashes_its_rows_carry() {
    let Some(installs) = installs("rpf8 names") else {
        return;
    };
    for (path, _) in installs
        .into_iter()
        .flatten()
        .filter(|(_, tag)| *tag == 0xFF)
    {
        let archive = open(&path).expect("a plain table opens");
        let mut spelled = 0;
        for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
            let name = node.name();
            if name.starts_with("hash_") {
                continue;
            }
            let hash = archive.entries[node.entry_index() as usize].name_offset();
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            assert!(
                hash == string_hash(name.as_bytes()) || hash == string_hash(stem.as_bytes()),
                "{name} does not hash to {hash:#010x}"
            );
            spelled += 1;
        }
        assert!(spelled > 0, "{} names none of its files", path.display());
    }
}

/// The write path on a real archive: rebuilt around every body as stored, then read again, the
/// archive lists the same files with the same bytes.
#[test]
fn a_plain_archive_rebuilt_around_its_stored_bodies_reads_back_the_same() {
    let Some(installs) = installs("rpf8 rebuild") else {
        return;
    };
    // The smallest plain archive keeps the copy in memory modest.
    let Some((path, _)) = installs
        .into_iter()
        .flatten()
        .filter(|(_, tag)| *tag == 0xFF)
        .min_by_key(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX))
    else {
        return;
    };
    let archive = open(&path).expect("a plain table opens");

    // An RPF8 archive carries every body as stored.
    let builder = archive.edit().expect("carried");
    let rebuilt = builder.to_bytes().expect("saved");
    assert_eq!(
        builder.planned_size().expect("planned"),
        rebuilt.len() as u64,
        "the preview is the size the save writes"
    );

    // The rows are the shipped ones but for where each body now lies: the hash, the encryption
    // bits, the extension id, the stored size and the compressor all come out as they went in.
    let rows_end = 16 + 256 + 24 * archive.file_count();
    let mut shipped = vec![0u8; rows_end];
    std::io::Read::read_exact(&mut std::fs::File::open(&path).expect("open"), &mut shipped)
        .expect("the shipped table");
    assert_eq!(rebuilt[..16], shipped[..16], "the header");
    assert_eq!(
        rebuilt.len() as u64,
        std::fs::metadata(&path).expect("metadata").len(),
        "the same bodies, padding and names take the same room"
    );
    const OFFSET_BITS: u64 = 0x7FFF_FFFF << 28;
    for (mine, theirs) in rebuilt[272..rows_end]
        .as_chunks::<24>()
        .0
        .iter()
        .zip(shipped[272..].as_chunks::<24>().0.iter())
    {
        let q = |row: &[u8], at: usize| u64::from_le_bytes(row[at..at + 8].try_into().unwrap());
        assert_eq!(q(mine, 0), q(theirs, 0), "the first word of a row");
        assert_eq!(
            q(mine, 8) & !OFFSET_BITS,
            q(theirs, 8) & !OFFSET_BITS,
            "its sizes"
        );
        assert_eq!(q(mine, 16), q(theirs, 16), "its size or flags");
    }

    let again = RpfArchive::from_bytes(rebuilt).expect("the rebuild reads");
    assert_eq!(again.version, RpfVersion::Rpf8);
    assert_eq!(again.file_count(), archive.file_count());
    for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
        let a = archive.entries[node.entry_index() as usize].clone();
        let other = again
            .nodes
            .iter()
            .find(|n| n.path() == node.path())
            .unwrap_or_else(|| panic!("{} is gone", node.path()));
        let b = again.entries[other.entry_index() as usize].clone();
        let (RpfEntry::Binary(a), RpfEntry::Binary(b)) = (a, b) else {
            continue;
        };
        assert_eq!(
            (a.uncompressed_size, a.consumed_size, a.compressor, a.tag),
            (b.uncompressed_size, b.consumed_size, b.compressor, b.tag),
            "{}",
            node.path()
        );
        let first = archive
            .read_form(node.entry_index(), BodyForm::AsStored)
            .expect("stored");
        let second = again
            .read_form(other.entry_index(), BodyForm::AsStored)
            .expect("stored");
        assert!(first == second, "{} differs after the rebuild", node.path());
    }
}
