//! `rpf9 find`: every entry of every archive under an install folder, nested archives included,
//! whose name holds a substring, whose hash is given, or whose kind matches; one line per entry.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::gta6_kinds::{entry_kinds, CLASSES, TYPES};
use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys, NameResolver};

use super::{
    build_archives, load_keys, load_resolver, take_common, usage, Common, CENSUS_MAX_DEPTH,
};

/// What an entry must match; every criterion given must hold.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Query {
    /// A substring of the entry's display name, compared without ASCII case.
    pub(super) name: Option<String>,
    /// The entry's name64, or its name32 where the value fits one.
    pub(super) hash: Option<u64>,
    pub(super) kind: Option<Kind>,
}

/// The kind of entry `--kind` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// A nested archive, by the clear header its body begins with.
    Archive,
    /// A 24-byte row's class byte.
    Class(u8),
    /// A kind label a row's class byte or type code names.
    Label(&'static str),
}

/// One matching entry: the archive file, the nested archives walked to reach it, the entry's name
/// and its real size where the row states one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hit {
    pub(super) archive: PathBuf,
    pub(super) inner: Vec<String>,
    pub(super) entry: String,
    pub(super) size: Option<u64>,
    pub(super) kind: String,
}

/// The matches of a search and how many archives or nested tables could not be opened.
#[derive(Debug, Default)]
pub(super) struct Found {
    pub(super) hits: Vec<Hit>,
    pub(super) archives: usize,
    pub(super) refused: usize,
}

pub(super) fn run(args: &[String]) -> ExitCode {
    let mut common = Common::default();
    let mut root: Option<&str> = None;
    let mut query = Query::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match take_common(&mut common, arg, &mut it) {
            Ok(true) => continue,
            Err(code) => return code,
            Ok(false) => {}
        }
        match arg.as_str() {
            "--name" => match it.next() {
                Some(v) => query.name = Some(v.to_ascii_lowercase()),
                None => return usage("--name needs a substring"),
            },
            "--hash" => match it.next().map(|v| parse_hash(v)) {
                Some(Ok(hash)) => query.hash = Some(hash),
                Some(Err(e)) => return usage(&e),
                None => return usage("--hash needs a hex value"),
            },
            "--kind" => match it.next().map(|v| parse_kind(v)) {
                Some(Ok(kind)) => query.kind = Some(kind),
                Some(Err(e)) => return usage(&e),
                None => return usage("--kind needs archive, a kind label or a class byte"),
            },
            other if other.starts_with("--") => return usage(&format!("unknown option {other}")),
            other => {
                if root.replace(other).is_some() {
                    return usage("give one install folder");
                }
            }
        }
    }
    let Some(root) = root else {
        return usage("give an install folder to search");
    };
    if query == Query::default() {
        return usage("give --name, --hash or --kind");
    }

    let keys = match load_keys(common.keys_path.as_deref(), common.exe_path.as_deref()) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let resolver = load_resolver(common.names_dir.as_deref());
    match do_find(Path::new(root), &query, &keys, &resolver) {
        Ok(found) => {
            for hit in &found.hits {
                let size = hit.size.map_or_else(|| "-".to_string(), |s| s.to_string());
                println!(
                    "{}\t{}\t{}\t{size}\t{}",
                    hit.archive.display(),
                    hit.inner.join("/"),
                    hit.entry,
                    hit.kind
                );
            }
            eprintln!(
                "{} match(es) in {} archive(s); {} table(s) could not be opened",
                found.hits.len(),
                found.archives,
                found.refused
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A hash as `--hash` takes it: hexadecimal, with an optional `0x` or `hash_` prefix.
pub(super) fn parse_hash(text: &str) -> Result<u64, String> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix("hash_"))
        .unwrap_or(text);
    u64::from_str_radix(digits, 16).map_err(|e| format!("--hash {text:?} is not hexadecimal: {e}"))
}

/// A kind as `--kind` takes it: `archive`, `resource`, `media`, a class byte in hex (`0x0e`), or a
/// kind label such as `drawable` or `height map`.
pub(super) fn parse_kind(text: &str) -> Result<Kind, String> {
    let labels = (CLASSES.iter().map(|(_, l)| *l)).chain(TYPES.iter().map(|(_, l)| *l));
    let label = labels.clone().find(|l| l.eq_ignore_ascii_case(text.trim()));
    match text.to_ascii_lowercase().as_str() {
        "archive" => Ok(Kind::Archive),
        "resource" => Ok(Kind::Class(0x0B)),
        "media" => Ok(Kind::Class(0xFF)),
        other => (other.strip_prefix("0x"))
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .map(Kind::Class)
            .or(label.map(Kind::Label))
            .ok_or_else(|| {
                let mut known: Vec<&str> = labels.collect();
                known.sort_unstable();
                known.dedup();
                format!(
                    "--kind {text:?} is not archive, a byte like 0x0e or a kind: {}",
                    known.join(", ")
                )
            }),
    }
}

/// Whether `entry` of kind label `kind` matches every criterion `query` gives; `is_archive` is
/// whether its body begins a nested archive.
pub(super) fn matches(query: &Query, entry: &Gta6Entry, kind: &str, is_archive: bool) -> bool {
    let name = query.name.as_ref().is_none_or(|sub| {
        entry
            .display_name()
            .to_ascii_lowercase()
            .contains(sub.as_str())
    });
    let hash = query.hash.is_none_or(|h| {
        entry.name64 == h || u32::try_from(h).is_ok_and(|h32| entry.name32 == Some(h32))
    });
    let of_kind = query.kind.is_none_or(|wanted| match wanted {
        Kind::Archive => is_archive,
        Kind::Class(byte) => entry.class == Some(byte),
        Kind::Label(label) => kind == label,
    });
    name && hash && of_kind
}

/// Search `root` (one archive, or every archive of an install folder) for the entries `query`
/// matches, walking into every nested archive.
pub(super) fn do_find(
    root: &Path,
    query: &Query,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Found, String> {
    let archives = if root.is_file() {
        vec![root.to_path_buf()]
    } else {
        build_archives(root)
    };
    if archives.is_empty() {
        return Err(format!("no .rpf under {}", root.display()));
    }
    let mut found = Found {
        archives: archives.len(),
        ..Found::default()
    };
    for path in &archives {
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        match Gta6Mount::open_root(&mut file, keys) {
            Ok(mut mount) => {
                mount.archive.resolve_names(resolver);
                let mut walk = Walk {
                    file: &mut file,
                    query,
                    keys,
                    resolver,
                    archive: path,
                    found: &mut found,
                };
                walk.mount(&mount, &mut Vec::new());
            }
            Err(e) => {
                eprintln!("  {}: {e}", path.display());
                found.refused += 1;
            }
        }
    }
    Ok(found)
}

/// One archive file's search, recursing through its nested archives.
struct Walk<'a> {
    file: &'a mut File,
    query: &'a Query,
    keys: &'a GtaViKeys,
    resolver: &'a NameResolver,
    archive: &'a Path,
    found: &'a mut Found,
}

impl Walk<'_> {
    fn mount(&mut self, mount: &Gta6Mount, inner: &mut Vec<String>) {
        let kinds = entry_kinds(&mount.archive);
        for (entry, kind) in mount.archive.entries.iter().zip(kinds) {
            let is_archive = match mount.entry_begins_archive(&mut *self.file, entry) {
                Ok(is_archive) => is_archive,
                Err(_) => {
                    self.found.refused += 1;
                    false
                }
            };
            if matches(self.query, entry, &kind, is_archive) {
                self.found.hits.push(Hit {
                    archive: self.archive.to_path_buf(),
                    inner: inner.clone(),
                    entry: entry.display_name(),
                    size: entry.real,
                    kind: kind.into_owned(),
                });
            }
            if !is_archive {
                continue;
            }
            if inner.len() >= CENSUS_MAX_DEPTH {
                self.found.refused += 1;
                continue;
            }
            match mount.open_nested(&mut *self.file, entry, self.keys) {
                Ok(mut child) => {
                    child.archive.resolve_names(self.resolver);
                    inner.push(entry.display_name());
                    self.mount(&child, inner);
                    inner.pop();
                }
                Err(_) => self.found.refused += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(class: Option<u8>) -> Gta6Entry {
        Gta6Entry {
            index: 0,
            name64: 1,
            name32: None,
            name: None,
            start: None,
            stored: None,
            real: None,
            class,
            cipher: None,
        }
    }

    #[test]
    fn a_kind_label_is_taken_by_any_case_and_matches_the_rows_it_names() {
        assert_eq!(parse_kind("Height Map"), Ok(Kind::Label("height map")));
        assert_eq!(parse_kind("drawable"), Ok(Kind::Label("drawable")));
        let why = parse_kind("drawables").expect_err("no such kind");
        assert!(
            why.contains("span grid"),
            "the error lists the kinds: {why}"
        );
        let query = Query {
            kind: Some(Kind::Label("drawable")),
            ..Query::default()
        };
        assert!(matches(&query, &entry(None), "drawable", false));
        assert!(!matches(&query, &entry(None), "cloth", false));
        let class = Query {
            kind: Some(Kind::Class(0x69)),
            ..Query::default()
        };
        assert!(matches(&class, &entry(Some(0x69)), "height map", false));
        assert!(!matches(&class, &entry(Some(0x68)), "span grid", false));
    }
}
