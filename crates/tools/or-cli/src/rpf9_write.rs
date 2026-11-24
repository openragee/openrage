//! `rpf9 copy|replace|remove|rename|defrag`: write a new GTA VI (RPF9) archive from an existing one.
//!
//!   `rpf9 copy <archive> --out <new>`      decipher and re-encipher each table, copy every body
//!   `rpf9 replace <archive> [inner/path] --entry <selector> --with <file> --out <new>`
//!   `rpf9 remove <archive> [inner/path] --entry <selector> --out <new>`
//!   `rpf9 rename <archive> [inner/path] --entry <selector> --name <path> --out <new>`
//!   `rpf9 defrag <archive> --out <new>`    pack the bodies in order; only starts and the table change
//!
//! The source is only read: `--out` names a new file. An `inner/path` walks nested archives as
//! `rpf9 list` does, and the selector names the entry the same way: a recovered name, a
//! `0x…`/`hash_…` hash or a decimal index. The writer is structural: the game verifies a signature
//! over a table when it mounts an archive, and only the game's maker can sign one, so every write
//! that changes a table says the game refuses the result.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::{
    Gta6Entry, Gta6Mount, GtaViKeys, NameResolver, Rpf9ArchiveWriter, GTA6_SNAPSHOT_ENV,
    GTA6_SNAPSHOT_FILE_NAME,
};
use or_io::gta6::names::{vocabulary_file, Vocabulary};

/// The folder of recovered-names tables used when `--names` is not given.
const NAMES_ENV: &str = "OPENRAGE_GTA6_NAMES";

const USAGE: &str = "\
usage:
  openrage-cli rpf9 copy <archive.rpf> --out <new.rpf> [--keys <snapshot>]
  openrage-cli rpf9 replace <archive.rpf> [inner/path] --entry <selector> --with <file> --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 remove <archive.rpf> [inner/path] --entry <selector> --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 rename <archive.rpf> [inner/path] --entry <selector> --name <path> --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 defrag <archive.rpf> --out <new.rpf> [--keys <snapshot>]";

/// What a write that changed a table says of the archive it wrote.
const STALE: &str = "Its table signature is stale: the game verifies a signature over the table \
when it mounts an archive, and only the game's maker can sign one, so the game refuses this \
archive. It is a structural copy.";

/// What a write that changed no table says.
const UNCHANGED: &str =
    "No table or body changed, so every table and its signature are the source archive's.";

#[derive(Default)]
struct Options {
    out: Option<PathBuf>,
    entry: Option<String>,
    with: Option<PathBuf>,
    name: Option<String>,
    names_dir: Option<PathBuf>,
    keys_path: Option<PathBuf>,
}

fn usage(msg: &str) -> ExitCode {
    eprintln!("Error: {msg}");
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

pub(crate) fn run(verb: &str, args: &[String]) -> ExitCode {
    let mut options = Options::default();
    let mut positionals: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let slot = match arg.as_str() {
            "--out" => &mut options.out,
            "--with" => &mut options.with,
            "--names" => &mut options.names_dir,
            "--keys" => &mut options.keys_path,
            "--entry" | "--name" => {
                let Some(value) = it.next() else {
                    return usage(&format!("{arg} needs a value"));
                };
                if arg == "--entry" {
                    options.entry = Some(value.clone());
                } else {
                    options.name = Some(value.clone());
                }
                continue;
            }
            other if other.starts_with("--") => return usage(&format!("unknown option {other}")),
            other => {
                positionals.push(other);
                continue;
            }
        };
        match it.next() {
            Some(value) => *slot = Some(PathBuf::from(value)),
            None => return usage(&format!("{arg} needs a value")),
        }
    }
    let Some((&archive, inner)) = positionals.split_first() else {
        return usage("give an archive to write from");
    };
    let inner: Vec<String> = inner
        .iter()
        .flat_map(|p| p.split(['/', '\\']))
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    match write(verb, Path::new(archive), &inner, &options) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn write(
    verb: &str,
    archive: &Path,
    inner: &[String],
    options: &Options,
) -> Result<String, String> {
    let out = options
        .out
        .as_deref()
        .ok_or("--out names the new archive to write")?;
    if same_file(archive, out) {
        return Err("--out must name a new file; the source archive is never written".into());
    }
    let keys = load_keys(options.keys_path.as_deref())?;
    let mut file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;

    let mut writer = Rpf9ArchiveWriter::new();
    match verb {
        "copy" | "defrag" => {
            if !inner.is_empty() || options.entry.is_some() {
                return Err(format!("{verb} takes the whole archive, not an entry"));
            }
            if verb == "defrag" {
                writer.defragment();
            }
        }
        _ => {
            let selector = options
                .entry
                .as_deref()
                .ok_or("--entry names the entry to edit")?;
            let resolver = load_resolver(options.names_dir.as_deref());
            let path = entry_path(&mut file, inner, selector, &keys, &resolver)?;
            match verb {
                "replace" => {
                    let with = options
                        .with
                        .as_deref()
                        .ok_or("--with names the file holding the new body")?;
                    let body =
                        std::fs::read(with).map_err(|e| format!("{}: {e}", with.display()))?;
                    writer.replace(&path, body);
                }
                "remove" => {
                    writer.remove(&path);
                }
                _ => {
                    let name = options
                        .name
                        .as_deref()
                        .ok_or("--name gives the entry's new path")?;
                    writer.rename(&path, name);
                }
            }
        }
    }

    let plan = writer.plan(&mut file, &keys).map_err(|e| e.to_string())?;
    let written = {
        // `create_new` refuses an existing file, a hard link to the source among them, so the
        // cleanup below only ever deletes a file this command made.
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out)
            .map_err(|e| format!("{}: {e}", out.display()))?;
        let mut sink = BufWriter::new(created);
        plan.write_to(&mut file, &mut sink)
            .map_err(|e| e.to_string())
            .and_then(|n| sink.flush().map(|()| n).map_err(|e| e.to_string()))
    };
    let written = match written {
        Ok(n) => n,
        Err(e) => {
            // A partial archive is not left behind under the name asked for.
            let _ = std::fs::remove_file(out);
            return Err(format!("{}: {e}", out.display()));
        }
    };
    let verdict = match (plan.tables_changed, plan.bodies_changed) {
        (0, 0) => UNCHANGED.to_owned(),
        (0, bodies) => format!("{bodies} body(ies) rewritten; no table changed."),
        (tables, _) => format!("{tables} table(s) rewritten. {STALE}"),
    };
    Ok(format!(
        "{}: {written} bytes written. {verdict}",
        out.display()
    ))
}

/// Whether `a` and `b` are the same existing file.
fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// The entry indices from the top-level table to the entry `selector` names inside `inner`.
fn entry_path(
    file: &mut File,
    inner: &[String],
    selector: &str,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Vec<u32>, String> {
    let mut mount = Gta6Mount::open_root(file, keys).map_err(|e| e.to_string())?;
    mount.archive.resolve_names(resolver);
    let mut path = Vec::with_capacity(inner.len() + 1);
    for component in inner {
        let entry = find_entry(&mount, component)
            .ok_or_else(|| format!("no entry {component:?} in the archive"))?;
        if !mount
            .entry_begins_archive(file, &entry)
            .map_err(|e| e.to_string())?
        {
            return Err(format!("{component:?} is an entry, not a nested archive"));
        }
        path.push(entry.index);
        mount = mount
            .open_nested(file, &entry, keys)
            .map_err(|e| e.to_string())?;
        mount.archive.resolve_names(resolver);
    }
    let entry =
        find_entry(&mount, selector).ok_or_else(|| format!("no entry {selector:?} there"))?;
    path.push(entry.index);
    Ok(path)
}

/// The entry `selector` names: its recovered name, its `0x…`/`hash_…` hash, or its decimal index.
fn find_entry(mount: &Gta6Mount, selector: &str) -> Option<Gta6Entry> {
    let matches = |entry: &&Gta6Entry| {
        let hex = format!("{:016x}", entry.name64);
        entry.name.as_deref() == Some(selector)
            || selector.eq_ignore_ascii_case(&format!("0x{hex}"))
            || selector.eq_ignore_ascii_case(&format!("hash_{hex}"))
            || selector.parse::<u32>().ok() == Some(entry.index)
    };
    mount.archive.entries.iter().find(matches).cloned()
}

/// The key snapshot: the one `--keys` names, else the one a session supplies for the install.
fn load_keys(explicit: Option<&Path>) -> Result<GtaViKeys, String> {
    if let Some(path) = explicit {
        return GtaViKeys::from_file(path).map_err(|e| e.to_string());
    }
    GtaViKeys::for_install(or_title::config::config_dir().as_deref())
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!(
                "No GTA VI key snapshot: set {GTA6_SNAPSHOT_ENV}, pass --keys, or place \
                 {GTA6_SNAPSHOT_FILE_NAME} beside the keys cache."
            )
        })
}

/// The recovered-names corpus from `dir` (or [`NAMES_ENV`]) and the config dir's vocabulary; empty
/// when neither is present, so entries are then named by hash or index.
fn load_resolver(dir: Option<&Path>) -> NameResolver {
    let mut resolver = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(NAMES_ENV).map(PathBuf::from))
        .map(|dir| NameResolver::load(&dir))
        .unwrap_or_default();
    if let Some(config) = or_title::config::config_dir() {
        resolver.merge_vocabulary(&Vocabulary::load(&vocabulary_file(&config)));
    }
    resolver
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_archive_is_never_the_output() {
        let name = format!("openrage-rpf9-write-{}.rpf", std::process::id());
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, b"9FPR").unwrap();
        let options = Options {
            out: Some(path.clone()),
            ..Options::default()
        };
        let refused = write("copy", &path, &[], &options);
        std::fs::remove_file(&path).unwrap();
        assert!(refused.unwrap_err().contains("never written"));
    }
}
