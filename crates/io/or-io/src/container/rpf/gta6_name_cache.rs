use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use super::{parse_hash, NameResolver};
use crate::gta6::hash::name_hash64;

fn valid(hash: u64, name: &str) -> bool {
    !name.is_empty()
        && !name.bytes().any(|b| b < b' ' || b == 127)
        && name_hash64(name.as_bytes()) == hash
}

pub(super) fn read_cache(directory: &Path) -> io::Result<BTreeMap<u64, String>> {
    let text = match fs::read_to_string(directory.join("resolved-names.tsv")) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error),
    };
    let mut names = BTreeMap::new();
    let mut rejected = 0;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let Some((hash, name)) = line.split_once('\t') else {
            rejected += 1;
            continue;
        };
        match parse_hash(hash).filter(|&hash| valid(hash, name)) {
            Some(hash) => match names.entry(hash) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(name.to_string());
                }
                // The first name stays; a second name for the same hash is one more rejected row.
                std::collections::btree_map::Entry::Occupied(slot) => {
                    if !slot.get().eq_ignore_ascii_case(name) {
                        rejected += 1;
                    }
                }
            },
            None => rejected += 1,
        }
    }
    if rejected != 0 {
        tracing::warn!(
            rejected,
            "ignored invalid rows in the local GTA VI names cache"
        );
    }
    Ok(names)
}

impl NameResolver {
    /// Atomically merges verified names into the local TSV without overwriting a conflicting name.
    pub fn cache_names<'a>(
        directory: &Path,
        found: impl IntoIterator<Item = (u64, &'a str)>,
    ) -> io::Result<usize> {
        let mut additions = BTreeMap::new();
        for (hash, name) in found {
            if !valid(hash, name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a cached name does not match its hash",
                ));
            }
            if additions
                .insert(hash, name)
                .is_some_and(|old| !old.eq_ignore_ascii_case(name))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "conflicting cached names",
                ));
            }
        }
        if additions.is_empty() {
            return Ok(0);
        }
        fs::create_dir_all(directory)?;
        // The advisory lock serializes readers that merge then replace, across threads and processes.
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(".resolved-names.lock"))?;
        lock.lock()?;
        let mut names = read_cache(directory)?;
        let before = names.len();
        for (hash, name) in additions {
            if let Some(old) = names.get(&hash) {
                if !old.eq_ignore_ascii_case(name) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "conflicting cached names",
                    ));
                }
            } else {
                names.insert(hash, name.to_string());
            }
        }
        let added = names.len() - before;
        if added == 0 {
            return Ok(0);
        }
        let destination = directory.join("resolved-names.tsv");
        let temporary = directory.join(".resolved-names.tsv.tmp");
        let result = (|| {
            let mut output = io::BufWriter::new(fs::File::create(&temporary)?);
            for (hash, name) in names {
                writeln!(output, "0x{hash:016x}\t{name}")?;
            }
            output.flush()?;
            output.get_ref().sync_all()?;
            drop(output);
            fs::rename(&temporary, &destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(added)
    }
}
