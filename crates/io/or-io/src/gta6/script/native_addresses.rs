//! The natives a folder of lifted listings declares, keyed by the executable address a compiled
//! script's `NATIVE` selector names.
//!
//! ⚠ **Nothing here is shipped.** The table is built at run time from the user's listings and
//! cached in their configuration folder; a name comes from the user's names registry by hash, else
//! from the listing.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::ir::Callee;
use super::listing::{Listing, ListingError, Target};
use super::Names;
use crate::gta5::script::natives::{NativeDb, NativeName};

/// The listings a folder is read for.
pub const LISTING_SUFFIX: &str = ".ysc.cs";
/// The cache's file in the user's configuration folder.
pub const CACHE_FILE: &str = "gta6_native_addresses.tsv";
/// The cache's first line, before the fingerprint of the folder it was built from.
const CACHE_HEAD: &str = "# openrage gta6 native addresses v1 ";
/// The label of the natives list made of the names the listings declare.
pub const LISTINGS_LABEL: &str = "the lifted listings";

/// Why a folder of listings gave no table.
#[derive(Debug, thiserror::Error)]
pub enum NativeAddressesError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{listing}: {source}")]
    Listing {
        listing: String,
        source: ListingError,
    },
    #[error("{} holds no `*{LISTING_SUFFIX}` listing", folder.display())]
    Empty { folder: PathBuf },
    #[error("{listing}: the native at {address:#x} {what}")]
    Conflict {
        listing: String,
        address: u32,
        what: String,
    },
}

/// What a listing declares a native at one address to be.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Declared {
    hash: u64,
    /// The listing's name, `_0x` and the hash where it has none.
    name: String,
}

/// The natives by address, and the names registry consulted by their hash.
#[derive(Clone, Debug, Default)]
pub struct NativeAddresses {
    by_address: HashMap<u32, Declared>,
    pub names: Names,
}

impl NativeAddresses {
    /// The table of `folder`, from the cache in `config_dir` where it was built from the same files, else read and cached there.
    pub fn load(folder: &Path, config_dir: Option<&Path>) -> Result<Self, NativeAddressesError> {
        let listings = listings(folder)?;
        let head = format!("{CACHE_HEAD}{:016x}", fingerprint(folder, &listings));
        let cached = config_dir.and_then(|dir| {
            let text = std::fs::read_to_string(dir.join(CACHE_FILE)).ok()?;
            let rows = text.strip_prefix(head.as_str())?.strip_prefix('\n')?;
            from_cache(rows)
        });
        if let Some(table) = cached {
            return Ok(table);
        }
        let table = Self::read(&listings)?;
        if let Some(dir) = config_dir {
            let at = dir.join(CACHE_FILE);
            let written = std::fs::create_dir_all(dir)
                .and_then(|()| std::fs::write(&at, table.cache_text(&head)));
            // A table that cannot be cached is still the table.
            if let Err(e) = written {
                tracing::warn!("the native address cache {}: {e}", at.display());
            }
        }
        Ok(table)
    }

    /// The table last cached in `config_dir`, whatever folder it was built from; `None` where there is none or it does not read.
    pub fn cached(config_dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(config_dir.join(CACHE_FILE)).ok()?;
        let (head, rows) = text.split_once('\n')?;
        head.starts_with(CACHE_HEAD).then(|| from_cache(rows))?
    }

    /// The table the listings at `paths` declare; a listing that does not open is refused by name.
    pub fn read(paths: &[PathBuf]) -> Result<Self, NativeAddressesError> {
        let mut table = Self::default();
        for path in paths {
            let listing = file_name(path);
            let text =
                std::fs::read_to_string(path).map_err(|source| NativeAddressesError::Io {
                    path: path.clone(),
                    source,
                })?;
            let opened = Listing::open(&text).map_err(|source| NativeAddressesError::Listing {
                listing: listing.clone(),
                source,
            })?;
            for (_, wrapper) in opened.declarations().iter() {
                if let Target::Call(Callee::Native {
                    name, hash, rva, ..
                }) = &wrapper.target
                {
                    table.declare(&listing, *rva, *hash, name)?;
                }
            }
        }
        Ok(table)
    }

    /// Adds one declaration; a name replaces a held hash-only one, any other difference is refused.
    fn declare(
        &mut self,
        listing: &str,
        address: u32,
        hash: u64,
        name: &str,
    ) -> Result<(), NativeAddressesError> {
        let conflict = |what: String| NativeAddressesError::Conflict {
            listing: listing.to_owned(),
            address,
            what,
        };
        let Some(held) = self.by_address.get_mut(&address) else {
            let name = name.to_owned();
            self.by_address.insert(address, Declared { hash, name });
            return Ok(());
        };
        if held.hash != hash {
            let what = format!("is 0x{hash:016X} here and 0x{:016X} before", held.hash);
            return Err(conflict(what));
        }
        match (unnamed(&held.name), unnamed(name)) {
            (true, false) => held.name = name.to_owned(),
            (false, false) if held.name != name => {
                return Err(conflict(format!(
                    "is `{name}` here and `{}` before",
                    held.name
                )));
            }
            _ => {}
        }
        Ok(())
    }

    /// The hash the native at `address` is declared with.
    pub fn hash_of(&self, address: u32) -> Option<u64> {
        self.by_address.get(&address).map(|d| d.hash)
    }

    /// The name of the native at `address`: the names registry's for its hash, else the listing's; `None` for a hash-only one.
    pub fn name_of(&self, address: u32) -> Option<&str> {
        let declared = self.by_address.get(&address)?;
        match self.names.native(declared.hash) {
            Some(native) => Some(native.name.as_str()),
            None => (!unnamed(&declared.name)).then_some(declared.name.as_str()),
        }
    }

    /// Gives `names` the hash of each handler declared here that no supplied list states, and returns how many it gave.
    // ⚠ A list the user supplied stands: a handler it states keeps its hash, and a natives list
    // it holds keeps naming. Only where `names` holds no natives list do the listings' own names
    // become one, so this is called after the natives list is chosen.
    pub fn supply(&self, names: &mut Names) -> usize {
        let mut given = 0;
        for (address, declared) in &self.by_address {
            if let Entry::Vacant(place) = names.native_hashes.entry(*address) {
                place.insert(declared.hash);
                given += 1;
            }
        }
        if names.natives.is_none() {
            let named = self.by_address.values().filter(|d| !unnamed(&d.name));
            let rows: Vec<(u64, NativeName)> = named
                .map(|d| {
                    let name = NativeName {
                        name: d.name.clone(),
                        ..NativeName::default()
                    };
                    (d.hash, name)
                })
                .collect();
            if !rows.is_empty() {
                names.natives = Some(NativeDb::from_names(LISTINGS_LABEL, rows));
            }
        }
        given
    }

    /// How many addresses are declared.
    pub fn len(&self) -> usize {
        self.by_address.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_address.is_empty()
    }

    /// The cache file: `head`, then `address<TAB>hash<TAB>name` rows by address.
    fn cache_text(&self, head: &str) -> String {
        let mut rows: Vec<_> = self.by_address.iter().collect();
        rows.sort_by_key(|(address, _)| **address);
        let mut text = format!("{head}\n");
        for (address, d) in rows {
            text.push_str(&format!("{address:08x}\t{:016x}\t{}\n", d.hash, d.name));
        }
        text
    }
}

/// What the supplied lists call the native whose handler is at `rva`: its name where a natives list names its hash, else `_0x` and the hash; `None` where no list states the handler.
pub fn listed(names: &Names, rva: u32) -> Option<String> {
    let hash = names.native_at(rva)?;
    Some(match names.native(hash) {
        Some(native) => match &native.namespace {
            Some(namespace) => format!("{namespace}.{}", native.name),
            None => native.name.clone(),
        },
        None => format!("_0x{hash:016X}"),
    })
}

/// The table a cache's rows hold; `None` for a row that does not read, so the folder is read again.
fn from_cache(rows: &str) -> Option<NativeAddresses> {
    let mut table = NativeAddresses::default();
    for row in rows.lines() {
        let mut fields = row.split('\t');
        let address = u32::from_str_radix(fields.next()?, 16).ok()?;
        let hash = u64::from_str_radix(fields.next()?, 16).ok()?;
        let name = fields.next().filter(|n| !n.is_empty())?.to_owned();
        if fields.next().is_some() {
            return None;
        }
        table.by_address.insert(address, Declared { hash, name });
    }
    Some(table)
}

/// A name that is only the hash spelled `_0x..` names nothing.
fn unnamed(name: &str) -> bool {
    name.starts_with("_0x")
}

/// The `*.ysc.cs` listings directly in `folder`, sorted.
fn listings(folder: &Path) -> Result<Vec<PathBuf>, NativeAddressesError> {
    let entries = std::fs::read_dir(folder).map_err(|source| NativeAddressesError::Io {
        path: folder.to_path_buf(),
        source,
    })?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && file_name(path).ends_with(LISTING_SUFFIX))
        .collect();
    paths.sort();
    match paths.is_empty() {
        true => Err(NativeAddressesError::Empty {
            folder: folder.to_path_buf(),
        }),
        false => Ok(paths),
    }
}

/// What the cache is checked against: the folder and each listing's name, size and modification time.
// ⚠ The standard hasher may change between toolchains; that only costs a rebuild.
fn fingerprint(folder: &Path, listings: &[PathBuf]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(folder.to_string_lossy().as_bytes());
    for path in listings {
        hasher.write(file_name(path).as_bytes());
        let meta = std::fs::metadata(path).ok();
        hasher.write_u64(meta.as_ref().map_or(0, |m| m.len()));
        let modified = meta.and_then(|m| m.modified().ok());
        let since = modified.and_then(|t| t.duration_since(UNIX_EPOCH).ok());
        hasher.write_u128(since.map_or(0, |d| d.as_nanos()));
    }
    hasher.finish()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
#[path = "native_addresses_tests.rs"]
mod tests;
