//! `rpf9 copy|replace|remove|rename|defrag|add|build`: write a new GTA VI (RPF9) archive.
//!
//!   `rpf9 copy <archive> --out <new>`      decipher and re-encipher each table, copy every body
//!   `rpf9 replace <archive> [inner/path] --entry <selector> --with <file> [--resource] --out <new>`
//!   `rpf9 remove <archive> [inner/path] --entry <selector> --out <new>`
//!   `rpf9 rename <archive> [inner/path] --entry <selector> --name <path> --out <new>`
//!   `rpf9 defrag <archive> --out <new>`    pack the bodies in order; only starts and the table change
//!   `rpf9 add <archive> [inner/path] --name <path> --with <file> [--type <code>] --out <new>`
//!   `rpf9 build <folder> --key-from <archive> --out <new>`  an archive of every file under the folder
//!
//! The source is only read: `--out` names a new file. An `inner/path` walks nested archives as
//! `rpf9 list` does, and the selector names the entry the same way: a recovered name, a
//! `0x…`/`hash_…` hash or a decimal index. `replace --resource` takes a headerless resource's loose
//! form, the file a resource save writes; `add --type` adds a loose resource of that type code,
//! and `add` without it stores the file chunked, or as a nested archive where it is one. `build`
//! takes its table's key from an archive of the same build. The writer is structural: the game
//! verifies a signature over a table when it mounts an archive, and only the game's maker can sign
//! one, so every write that changes a table says the game refuses the result.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
use or_io::container::rpf::{
    Gta6Entry, Gta6Mount, GtaViKeys, NameResolver, Rpf9ArchiveWriter, Rpf9Plan,
};
use or_io::gta6::names::{vocabulary_file, Vocabulary};

/// The bytes `9FPR` an RPF9 archive begins with.
const RPF9_MAGIC: [u8; 4] = *b"9FPR";

const USAGE: &str = "\
usage:
  openrage-cli rpf9 copy <archive.rpf> --out <new.rpf> [--keys <snapshot>]
  openrage-cli rpf9 replace <archive.rpf> [inner/path] --entry <selector> --with <file> [--resource] --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 remove <archive.rpf> [inner/path] --entry <selector> --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 rename <archive.rpf> [inner/path] --entry <selector> --name <path> --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 defrag <archive.rpf> --out <new.rpf> [--keys <snapshot>]
  openrage-cli rpf9 add <archive.rpf> [inner/path] --name <path> --with <file> [--type <code>] --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 build <folder> --key-from <archive.rpf> --out <new.rpf> [--keys <snapshot>]
  every verb also takes --exe <game_scarlett_master.exe>, the executable whose keys it reads with";

/// What a write that changed a table says of the archive it wrote.
const STALE: &str = "Its table signature is stale: the game verifies a signature over the table \
when it mounts an archive, and only the game's maker can sign one, so the game refuses this \
archive. It is a structural copy.";

/// What a build says of the archive it wrote.
const UNSIGNED: &str = "Its signature descriptor is empty: the game verifies a signature over \
the table when it mounts an archive, and only the game's maker can sign one, so the game refuses \
this archive. It is a structural build.";

/// What a write that changed no table says.
const UNCHANGED: &str =
    "No table or body changed, so every table and its signature are the source archive's.";

#[derive(Default)]
struct Options {
    out: Option<PathBuf>,
    entry: Option<String>,
    with: Option<PathBuf>,
    name: Option<String>,
    type_code: Option<String>,
    resource: bool,
    key_from: Option<PathBuf>,
    names_dir: Option<PathBuf>,
    keys_path: Option<PathBuf>,
    exe_path: Option<PathBuf>,
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
            "--key-from" => &mut options.key_from,
            "--names" => &mut options.names_dir,
            "--keys" => &mut options.keys_path,
            "--exe" => &mut options.exe_path,
            "--resource" => {
                options.resource = true;
                continue;
            }
            "--entry" | "--name" | "--type" => {
                let Some(value) = it.next() else {
                    return usage(&format!("{arg} needs a value"));
                };
                let field = match arg.as_str() {
                    "--entry" => &mut options.entry,
                    "--name" => &mut options.name,
                    _ => &mut options.type_code,
                };
                *field = Some(value.clone());
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
    let Some((&source, inner)) = positionals.split_first() else {
        return usage(if verb == "build" {
            "give a folder to build from"
        } else {
            "give an archive to write from"
        });
    };
    let inner: Vec<String> = inner
        .iter()
        .flat_map(|p| p.split(['/', '\\']))
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    let result = if verb == "build" {
        build_from(Path::new(source), &inner, &options)
    } else {
        write(verb, Path::new(source), &inner, &options)
    };
    match result {
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
    let keys = crate::gta6_keys::load(options.keys_path.as_deref(), options.exe_path.as_deref())?;
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
        "add" => {
            let name = options
                .name
                .as_deref()
                .ok_or("--name gives the new entry's path")?;
            let resolver = load_resolver(options.names_dir.as_deref());
            let (_, path) = walk(&mut file, inner, &keys, &resolver)?;
            let bytes = read_with(options)?;
            let body = match options.type_code.as_deref() {
                Some(code) => Rpf9NewBody::Resource {
                    type_code: parse_code(code)?,
                    loose: bytes,
                },
                None if bytes.starts_with(&RPF9_MAGIC) => Rpf9NewBody::Archive(bytes),
                None => Rpf9NewBody::Chunked(bytes),
            };
            writer.add(&path, name, body);
        }
        _ => {
            let selector = options
                .entry
                .as_deref()
                .ok_or("--entry names the entry to edit")?;
            let resolver = load_resolver(options.names_dir.as_deref());
            let path = entry_path(&mut file, inner, selector, &keys, &resolver)?;
            match verb {
                "replace" if options.resource => {
                    writer.replace_resource(&path, read_with(options)?, None);
                }
                "replace" => {
                    writer.replace(&path, read_with(options)?);
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
    let written = write_plan(&plan, &mut file, out)?;
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

/// `rpf9 build`: an archive of every file under `folder`, each named by its path below it.
fn build_from(folder: &Path, inner: &[String], options: &Options) -> Result<String, String> {
    if !inner.is_empty() {
        return Err("build takes one folder".into());
    }
    let out = options
        .out
        .as_deref()
        .ok_or("--out names the new archive to write")?;
    let like = options.key_from.as_deref().ok_or(
        "--key-from names an archive of the same build, whose table key the new one takes",
    )?;
    let keys = crate::gta6_keys::load(options.keys_path.as_deref(), options.exe_path.as_deref())?;
    let mut source = File::open(like).map_err(|e| format!("{}: {e}", like.display()))?;
    let mount = Gta6Mount::open_root(&mut source, &keys).map_err(|e| e.to_string())?;
    let key = Rpf9IndexKey::of(&mount.archive.toc).map_err(|e| e.to_string())?;

    let mut files = Vec::new();
    collect_files(folder, folder, &mut files)?;
    files.sort();
    let mut entries = Vec::with_capacity(files.len());
    for (name, path) in files {
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let body = if bytes.starts_with(&RPF9_MAGIC) {
            Rpf9NewBody::Archive(bytes)
        } else {
            Rpf9NewBody::Chunked(bytes)
        };
        entries.push((name, body));
    }
    let plan = build(&entries, &key, &keys).map_err(|e| e.to_string())?;
    let written = write_plan(&plan, &mut std::io::Cursor::new(Vec::<u8>::new()), out)?;
    Ok(format!(
        "{}: {} entries, {written} bytes written. {UNSIGNED}",
        out.display(),
        entries.len()
    ))
}

/// Every file under `dir`, each with its path below `root`, `/`-separated.
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    let listing = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for item in listing {
        let path = item.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if let Ok(below) = path.strip_prefix(root) {
            let name = (below.components())
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((name, path));
        }
    }
    Ok(())
}

/// Write `plan` to the new file `out`, copying unchanged runs from `src`; the bytes written.
fn write_plan<R: Read + Seek>(plan: &Rpf9Plan, src: &mut R, out: &Path) -> Result<u64, String> {
    let written = {
        // `create_new` refuses an existing file, a hard link to the source among them, so the
        // cleanup below only ever deletes a file this command made.
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out)
            .map_err(|e| format!("{}: {e}", out.display()))?;
        let mut sink = BufWriter::new(created);
        plan.write_to(src, &mut sink)
            .map_err(|e| e.to_string())
            .and_then(|n| sink.flush().map(|()| n).map_err(|e| e.to_string()))
    };
    written.map_err(|e| {
        // A partial archive is not left behind under the name asked for.
        let _ = std::fs::remove_file(out);
        format!("{}: {e}", out.display())
    })
}

/// The bytes of the file `--with` names.
fn read_with(options: &Options) -> Result<Vec<u8>, String> {
    let with = options
        .with
        .as_deref()
        .ok_or("--with names the file holding the new body")?;
    std::fs::read(with).map_err(|e| format!("{}: {e}", with.display()))
}

/// A resource type code, `0x…` hexadecimal or decimal.
fn parse_code(code: &str) -> Result<u32, String> {
    match code.strip_prefix("0x").or_else(|| code.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => code.parse(),
    }
    .map_err(|_| format!("--type {code:?} is not a type code"))
}

/// Whether `a` and `b` are the same existing file.
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// The archive `inner` names, walked from the top-level table, and its entry indices from there.
fn walk(
    file: &mut File,
    inner: &[String],
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<(Gta6Mount, Vec<u32>), String> {
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
    Ok((mount, path))
}

/// The entry indices from the top-level table to the entry `selector` names inside `inner`.
fn entry_path(
    file: &mut File,
    inner: &[String],
    selector: &str,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Vec<u32>, String> {
    let (mount, mut path) = walk(file, inner, keys, resolver)?;
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

/// Bundled names supplemented by `dir` or `OPENRAGE_GTA6_NAMES`, and the local vocabulary.
fn load_resolver(dir: Option<&Path>) -> NameResolver {
    let mut resolver = NameResolver::load_with_defaults(dir);
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

    #[test]
    fn a_type_code_reads_as_hexadecimal_or_decimal() {
        assert_eq!(parse_code("0x43"), Ok(0x43));
        assert_eq!(parse_code("0X2"), Ok(2));
        assert_eq!(parse_code("67"), Ok(67));
        assert!(parse_code("drawable").is_err());
    }

    #[test]
    fn a_build_names_each_file_by_its_path_below_the_folder() {
        let root = std::env::temp_dir().join(format!("openrage-rpf9-build-{}", std::process::id()));
        let nested = root.join("maps").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("top.txt"), b"a").unwrap();
        std::fs::write(nested.join("leaf.bin"), b"b").unwrap();
        let mut files = Vec::new();
        let listed = collect_files(&root, &root, &mut files);
        std::fs::remove_dir_all(&root).unwrap();
        listed.unwrap();
        files.sort();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["maps/deep/leaf.bin", "top.txt"]);
    }

    #[test]
    fn a_build_without_a_key_source_is_refused_before_anything_is_written() {
        let out =
            std::env::temp_dir().join(format!("openrage-rpf9-built-{}.rpf", std::process::id()));
        let options = Options {
            out: Some(out.clone()),
            ..Options::default()
        };
        let refused = build_from(Path::new("."), &[], &options);
        assert!(refused.unwrap_err().contains("--key-from"));
        assert!(!out.exists());
    }
}
