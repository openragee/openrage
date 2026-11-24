//! `rpf9`: the GTA VI (RPF9) archive explorer, over the decrypted table of contents.
//!
//!   `rpf9 list <archive> [inner/path]`     one row per entry: start, stored, real, name64, name
//!   `rpf9 tree …`                          an alias of `list`, kept for the earlier name
//!   `rpf9 extract <archive> <dest> …`      write entry bodies to disk, inflated, at their exact extent
//!                                          (with `--decrypt`, a content-ciphered body is deciphered first)
//!   `rpf9 census <build-root>`             walk every archive and nested table, count them, and
//!                                          classify each flagged body by its inflated head
//!
//! An `inner/path` of slash-separated components navigates nested archives from the entry table:
//! `data.rpf common.rpf` lists the `common.rpf` held inside `data.rpf`, recursively. A component is
//! matched against an entry by its recovered name, its `0x…`/`hash_…` hash or its decimal index.
//!
//! The key snapshot and the recovered-names corpus are game-derived and read at runtime; neither is
//! committed. Without a snapshot the archive cannot be deciphered and the command says so; without
//! the names corpus every entry shows as its hash.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys, NameResolver};
use or_io::gta6::names::{vocabulary_file, Vocabulary};

/// The environment variable that names a folder of recovered-names tables (`vocabulary.txt`,
/// `resolved-names.tsv`, `toc-entries-resolved.tsv`), used when `--names` is not given.
const NAMES_ENV: &str = "OPENRAGE_GTA6_NAMES";

pub(crate) fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("list" | "tree") => list(&args[1..]),
        Some("extract") => extract(&args[1..]),
        Some("census") => census(&args[1..]),
        Some(verb @ ("copy" | "replace" | "remove" | "rename" | "defrag")) => {
            crate::rpf9_write::run(verb, &args[1..])
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

const USAGE: &str = "\
usage:
  openrage-cli rpf9 list <archive.rpf> [inner/path] [--names <dir>] [--keys <snapshot>] [--groups] [--nested] [--json]
  openrage-cli rpf9 extract <archive.rpf> <dest-dir> [inner/path] [--entry <selector>] [--filter <substr>] [--decrypt] [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 census <build-root> [--names <dir>] [--keys <snapshot>] [--out <dir>] [--top-only]
  openrage-cli rpf9 copy <archive.rpf> --out <new.rpf> [--keys <snapshot>]
  openrage-cli rpf9 replace <archive.rpf> [inner/path] --entry <selector> --with <file> --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 remove <archive.rpf> [inner/path] --entry <selector> --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 rename <archive.rpf> [inner/path] --entry <selector> --name <path> --out <new.rpf> [--names <dir>] [--keys <snapshot>]
  openrage-cli rpf9 defrag <archive.rpf> --out <new.rpf> [--keys <snapshot>]";

fn usage(msg: &str) -> ExitCode {
    eprintln!("Error: {msg}");
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

// --- shared option parsing --------------------------------------------------

/// The options every verb shares: the names folder and the key snapshot override.
#[derive(Default)]
struct Common {
    names_dir: Option<PathBuf>,
    keys_path: Option<PathBuf>,
}

/// Consume a `--names`/`--keys` option from `arg`, reading its value from `it`; `Ok(true)` when it
/// was one of them, `Ok(false)` when `arg` is for the caller to handle, `Err` on a missing value.
fn take_common<'a>(
    common: &mut Common,
    arg: &str,
    it: &mut impl Iterator<Item = &'a String>,
) -> Result<bool, ExitCode> {
    match arg {
        "--names" => match it.next() {
            Some(dir) => common.names_dir = Some(PathBuf::from(dir)),
            None => return Err(usage("--names needs a folder")),
        },
        "--keys" => match it.next() {
            Some(path) => common.keys_path = Some(PathBuf::from(path)),
            None => return Err(usage("--keys needs a snapshot file")),
        },
        _ => return Ok(false),
    }
    Ok(true)
}

/// The key snapshot: the one `--keys` names, else the one a session supplies for the install.
fn load_keys(explicit: Option<&Path>) -> Result<GtaViKeys, String> {
    if let Some(path) = explicit {
        return GtaViKeys::from_file(path).map_err(|e| e.to_string());
    }
    match GtaViKeys::for_install(or_title::config::config_dir().as_deref())
        .map_err(|e| e.to_string())?
    {
        Some(keys) => Ok(keys),
        None => Err(format!(
            "No GTA VI key snapshot: set {}, pass --keys, or place {} beside the keys cache.",
            or_io::container::rpf::GTA6_SNAPSHOT_ENV,
            or_io::container::rpf::GTA6_SNAPSHOT_FILE_NAME,
        )),
    }
}

/// Load the recovered-names corpus from `dir` (or the folder [`NAMES_ENV`] names): the `vocabulary.txt`
/// hashed at load, plus the per-build tables as a shortcut. An empty resolver is returned when the
/// folder is absent, so a missing corpus never blocks — entries show as their hash.
fn load_resolver(dir: Option<&Path>) -> NameResolver {
    let mut resolver = match dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(NAMES_ENV).map(PathBuf::from))
    {
        Some(dir) => NameResolver::load(&dir),
        None => NameResolver::new(),
    };
    // Also fold in the vocabulary the app harvested into the config dir, so names already found in the
    // user's own archives resolve here, not only those under the names folder or the env var.
    if let Some(config) = or_title::config::config_dir() {
        resolver.merge_vocabulary(&Vocabulary::load(&vocabulary_file(&config)));
    }
    resolver
}

// --- mounting and the inner path --------------------------------------------

/// Open the top-level archive at `path` and walk `inner` (slash-separated nested-archive selectors)
/// into it, returning the mount reached. Each component must name a nested archive; the file stays
/// open in `file` for later body reads.
fn mount_path(
    file: &mut File,
    inner: &[String],
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Gta6Mount, String> {
    let mut mount = Gta6Mount::open_root(file, keys).map_err(|e| e.to_string())?;
    mount.archive.resolve_names(resolver);
    for component in inner {
        let entry = find_entry(&mount, component)
            .ok_or_else(|| format!("no entry {component:?} in the archive"))?;
        if !mount
            .entry_begins_archive(file, &entry)
            .map_err(|e| e.to_string())?
        {
            return Err(format!("{component:?} is an entry, not a nested archive"));
        }
        mount = mount
            .open_nested(file, &entry, keys)
            .map_err(|e| e.to_string())?;
        mount.archive.resolve_names(resolver);
    }
    Ok(mount)
}

/// The entry of `mount` a path component or selector names: by recovered name, by `0x…`/`hash_…`
/// hash, or by decimal index. Names are matched first, so a real path wins over a numeric name.
fn find_entry(mount: &Gta6Mount, selector: &str) -> Option<Gta6Entry> {
    mount
        .archive
        .entries
        .iter()
        .find(|e| entry_matches(e, selector))
        .cloned()
}

/// Whether `entry` is named by `selector`: its recovered name, its `0x{hash}` or `hash_{hash}` form,
/// or its decimal index.
fn entry_matches(entry: &Gta6Entry, selector: &str) -> bool {
    if entry.name.as_deref() == Some(selector) {
        return true;
    }
    let hex = format!("{:016x}", entry.name64);
    if selector.eq_ignore_ascii_case(&format!("0x{hex}"))
        || selector.eq_ignore_ascii_case(&format!("hash_{hex}"))
    {
        return true;
    }
    selector.parse::<u32>().ok() == Some(entry.index)
}

// --- list -------------------------------------------------------------------

fn list(args: &[String]) -> ExitCode {
    let mut common = Common::default();
    let mut positionals: Vec<&str> = Vec::new();
    let (mut show_groups, mut nested, mut json) = (false, false, false);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match take_common(&mut common, arg, &mut it) {
            Ok(true) => continue,
            Err(code) => return code,
            Ok(false) => {}
        }
        match arg.as_str() {
            "--groups" => show_groups = true,
            "--nested" => nested = true,
            "--json" => json = true,
            other if other.starts_with("--") => return usage(&format!("unknown option {other}")),
            other => positionals.push(other),
        }
    }
    let Some((&archive, inner)) = positionals.split_first() else {
        return usage("give an archive to list");
    };
    let inner: Vec<String> = inner.iter().flat_map(|p| split_inner(p)).collect();

    let keys = match load_keys(common.keys_path.as_deref()) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let resolver = load_resolver(common.names_dir.as_deref());
    if resolver.is_empty() {
        eprintln!("(no recovered names found; entries show as their hash)");
    }

    match do_list(
        Path::new(archive),
        &inner,
        &keys,
        &resolver,
        show_groups,
        nested,
        json,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn do_list(
    path: &Path,
    inner: &[String],
    keys: &GtaViKeys,
    resolver: &NameResolver,
    show_groups: bool,
    nested: bool,
    json: bool,
) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mount = mount_path(&mut file, inner, keys, resolver)?;

    if json {
        let value = entries_json(&mount.archive.entries);
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    let label = display_path(path, inner);
    println!("{label}: {}", counts(&mount));
    print_listing(
        &mount.archive.entries,
        mount.archive.groups(),
        show_groups,
        1,
    );

    if nested {
        // One level of nested archives, mounted from the table (no scan). Deeper nesting is reached
        // with a longer inner path.
        let children: Vec<Gta6Entry> = mount
            .archive
            .entries
            .iter()
            .filter(|e| mount.entry_begins_archive(&mut file, e).unwrap_or(false))
            .cloned()
            .collect();
        println!("  {} nested RPF9 archive(s):", children.len());
        for entry in &children {
            let mut child = mount
                .open_nested(&mut file, entry, keys)
                .map_err(|e| e.to_string())?;
            child.archive.resolve_names(resolver);
            println!("  {}: {}", entry.display_name(), counts(&child));
            print_listing(
                &child.archive.entries,
                child.archive.groups(),
                show_groups,
                2,
            );
        }
    }
    Ok(())
}

/// The entry-count line for a mount: how many entries the table lists and the index key.
fn counts(mount: &Gta6Mount) -> String {
    format!(
        "{} entries, index key {:#010X}",
        mount.archive.entries.len(),
        mount.archive.toc.key_id,
    )
}

fn display_path(path: &Path, inner: &[String]) -> String {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archive");
    if inner.is_empty() {
        name.to_string()
    } else {
        format!("{name}/{}", inner.join("/"))
    }
}

fn print_listing(
    entries: &[Gta6Entry],
    groups: &[or_io::container::rpf::Rpf9Group],
    show_groups: bool,
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    if show_groups {
        println!("{indent}groups:");
        for group in groups {
            println!(
                "{indent}  {:#018X}  count {:>6}  width {:>3}  {}  [{}, {})",
                group.name_hash,
                group.count,
                group.width,
                if group.external { "ext " } else { "main" },
                group.data.start,
                group.data.end,
            );
        }
        println!("{indent}entries:");
    }
    for entry in entries {
        let size = entry
            .size()
            .map(|s| format!("{s:>10}"))
            .unwrap_or_else(|| "         -".to_string());
        let cipher = if entry.cipher.is_some() {
            " [cipher]"
        } else {
            ""
        };
        println!(
            "{indent}  {:>6}  {:#018X}  {size}  {}{cipher}",
            entry.index,
            entry.name64,
            entry.display_name(),
        );
    }
}

/// The JSON array for a listing: one object per entry with its name, hash, extent and cipher flag.
fn entries_json(entries: &[Gta6Entry]) -> serde_json::Value {
    serde_json::Value::Array(entries.iter().map(entry_json).collect())
}

fn entry_json(entry: &Gta6Entry) -> serde_json::Value {
    serde_json::json!({
        "index": entry.index,
        "name": entry.name,
        "hash": format!("{:#018x}", entry.name64),
        "name32": entry.name32.map(|h| format!("{h:#010x}")),
        "start": entry.start,
        "stored": entry.stored,
        "real": entry.real,
        "cipher": entry.cipher.is_some(),
    })
}

// --- extract ----------------------------------------------------------------

fn extract(args: &[String]) -> ExitCode {
    let mut common = Common::default();
    let mut positionals: Vec<&str> = Vec::new();
    let (mut entry_sel, mut filter): (Option<String>, Option<String>) = (None, None);
    let mut decrypt = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match take_common(&mut common, arg, &mut it) {
            Ok(true) => continue,
            Err(code) => return code,
            Ok(false) => {}
        }
        match arg.as_str() {
            "--entry" => match it.next() {
                Some(v) => entry_sel = Some(v.clone()),
                None => return usage("--entry needs a selector"),
            },
            "--filter" => match it.next() {
                Some(v) => filter = Some(v.clone()),
                None => return usage("--filter needs a substring"),
            },
            "--decrypt" => decrypt = true,
            other if other.starts_with("--") => return usage(&format!("unknown option {other}")),
            other => positionals.push(other),
        }
    }
    let [archive, dest, inner @ ..] = positionals.as_slice() else {
        return usage("give an archive and a destination directory");
    };
    let inner: Vec<String> = inner.iter().flat_map(|p| split_inner(p)).collect();

    let keys = match load_keys(common.keys_path.as_deref()) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let resolver = load_resolver(common.names_dir.as_deref());

    match do_extract(
        Path::new(archive),
        Path::new(dest),
        &inner,
        entry_sel.as_deref(),
        filter.as_deref(),
        decrypt,
        &keys,
        &resolver,
    ) {
        Ok(n) => {
            println!(
                "Extracted {n} entr{} to {dest}",
                if n == 1 { "y" } else { "ies" }
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn do_extract(
    path: &Path,
    dest: &Path,
    inner: &[String],
    entry_sel: Option<&str>,
    filter: Option<&str>,
    decrypt: bool,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<usize, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mount = mount_path(&mut file, inner, keys, resolver)?;

    let chosen: Vec<Gta6Entry> = select_entries(&mount.archive.entries, entry_sel, filter);
    if chosen.is_empty() {
        return Err("no entry matched".to_string());
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;

    let mut written = 0usize;
    for entry in &chosen {
        // A content-ciphered body is written only with `--decrypt`, deciphered and inflated; without
        // the flag it is skipped, as the clear reader would write ciphertext. A clear entry reads the
        // same either way.
        if entry.cipher.is_some() {
            if !decrypt {
                eprintln!(
                    "  skipped {} (enciphered body; pass --decrypt to decipher it)",
                    entry.display_name()
                );
                continue;
            }
        } else if mount
            .entry_begins_archive(&mut file, entry)
            .unwrap_or(false)
        {
            eprintln!(
                "  skipped {} (a nested archive; list it with an inner path)",
                entry.display_name()
            );
            continue;
        }
        let read = if entry.cipher.is_some() {
            mount.read_entry_decrypted(&mut file, entry, keys)
        } else {
            mount.read_entry(&mut file, entry)
        };
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("  skipped {} ({e})", entry.display_name());
                continue;
            }
        };
        let out = dest.join(safe_out_name(entry));
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
        written += 1;
    }
    Ok(written)
}

/// The entries to extract: the one `--entry` names, those whose name holds `--filter`, or, with
/// neither, every entry.
fn select_entries(
    entries: &[Gta6Entry],
    entry_sel: Option<&str>,
    filter: Option<&str>,
) -> Vec<Gta6Entry> {
    entries
        .iter()
        .filter(|e| match (entry_sel, filter) {
            (Some(sel), _) => entry_matches(e, sel),
            (None, Some(sub)) => e.display_name().contains(sub),
            (None, None) => true,
        })
        .cloned()
        .collect()
}

/// A destination-relative path for `entry`, built from its display name so a game path cannot escape
/// the destination: back-slashes become slashes, and empty, `.` and `..` components are dropped.
fn safe_out_name(entry: &Gta6Entry) -> String {
    let name = entry.display_name().replace('\\', "/");
    let mut parts: Vec<&str> = name
        .split('/')
        .filter(|c| !c.is_empty() && *c != "." && *c != "..")
        .collect();
    // A name64-only fallback (`hash_…`) already has no separators; a wholly-stripped name (e.g. "..")
    // falls back to the hash so a file is still written.
    if parts.is_empty() {
        return format!("hash_{:016x}", entry.name64);
    }
    // Strip a Windows drive prefix such as "C:" that a stray name could carry.
    if let Some(first) = parts.first_mut() {
        if let Some((_, rest)) = first.split_once(':') {
            *first = rest;
        }
    }
    parts.retain(|c| !c.is_empty());
    if parts.is_empty() {
        return format!("hash_{:016x}", entry.name64);
    }
    // Map the characters Windows forbids in a path component to '_' in every component, so a ':' in a
    // later component cannot open an NTFS alternate data stream and a '?'/'*' cannot glob.
    parts
        .iter()
        .map(|c| sanitize_component(c))
        .collect::<Vec<_>>()
        .join("/")
}

/// A path component with the characters Windows forbids mapped to '_'; entry names are untrusted.
fn sanitize_component(component: &str) -> String {
    component
        .chars()
        .map(|c| {
            if matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

// --- census -----------------------------------------------------------------

#[derive(Default)]
struct CensusCounts {
    entries: usize,
    nested: usize,
    leaf: usize,
    cipher: usize,
    resolved: usize,
    /// Nested tables that could not be probed or opened, or that exceeded the depth cap: a count the
    /// report shows rather than silently dropping from the totals.
    refused: usize,
    /// The inflated-head classes of the content-ciphered (type 17) entries this mount holds.
    heads: HeadCounts,
}

/// How many content-ciphered bodies inflate to each recognised head: a PSO (`PSIN`), an `RBF0`, a
/// text/XML body (a `<` or UTF-8-BOM start, or a plain-text list), a headerless resource (a leading
/// 8-byte zero slot then a pointer — the script programs), anything else, and the bodies that would
/// not decrypt.
#[derive(Default)]
struct HeadCounts {
    psin: usize,
    rbf0: usize,
    text: usize,
    headerless: usize,
    other: usize,
    unreadable: usize,
}

impl HeadCounts {
    fn add(&mut self, other: &HeadCounts) {
        self.psin += other.psin;
        self.rbf0 += other.rbf0;
        self.text += other.text;
        self.headerless += other.headerless;
        self.other += other.other;
        self.unreadable += other.unreadable;
    }

    /// Classify one inflated body by its head, bumping the matching class.
    fn classify(&mut self, body: &[u8]) {
        if body.starts_with(b"PSIN") {
            self.psin += 1;
        } else if body.starts_with(b"RBF0") {
            self.rbf0 += 1;
        } else if body.len() >= 8 && body[..8].iter().all(|&b| b == 0) {
            // A headerless resource (a script program) opens with an 8-byte zero slot, then a pointer.
            self.headerless += 1;
        } else if is_text_head(body) {
            self.text += 1;
        } else {
            self.other += 1;
        }
    }
}

/// Whether a body's head reads as text: a `<` start, a UTF-8 BOM, or a run of printable ASCII (a
/// plain-text list). A binary head (a script program's different leading bytes) is not text.
fn is_text_head(body: &[u8]) -> bool {
    if body.first() == Some(&b'<') || body.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return true;
    }
    let n = body.len().min(32);
    n > 0
        && body[..n]
            .iter()
            .all(|&c| matches!(c, b'\t' | b'\r' | b'\n') || (0x20..=0x7E).contains(&c))
}

/// A self-referential or cyclic mount would recurse forever; real nesting is shallow, so a depth past
/// this is counted as a refusal rather than followed.
const CENSUS_MAX_DEPTH: usize = 8;

fn census(args: &[String]) -> ExitCode {
    let mut common = Common::default();
    let mut root: Option<&str> = None;
    let mut out: Option<PathBuf> = None;
    let mut top_only = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match take_common(&mut common, arg, &mut it) {
            Ok(true) => continue,
            Err(code) => return code,
            Ok(false) => {}
        }
        match arg.as_str() {
            "--out" => match it.next() {
                Some(dir) => out = Some(PathBuf::from(dir)),
                None => return usage("--out needs a folder"),
            },
            "--top-only" => top_only = true,
            other if other.starts_with("--") => return usage(&format!("unknown option {other}")),
            other => {
                if root.replace(other).is_some() {
                    return usage("give one build root");
                }
            }
        }
    }
    let Some(root) = root else {
        return usage("give a build root to census");
    };
    // The default output folder is the gitignored parity notes directory; the census never writes
    // into the tracked tree. The folder is created if absent.
    let out = out.unwrap_or_else(|| PathBuf::from("docs/extra/gta6/parity"));

    let keys = match load_keys(common.keys_path.as_deref()) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let resolver = load_resolver(common.names_dir.as_deref());

    match do_census(Path::new(root), &out, top_only, &keys, &resolver) {
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

fn do_census(
    root: &Path,
    out: &Path,
    top_only: bool,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<String, String> {
    let archives = build_archives(root);
    if archives.is_empty() {
        return Err(format!("no .rpf under {}", root.display()));
    }

    let mut report = String::new();
    report.push_str("# RPF9 install census\n\n");
    report.push_str(
        "Counts only, from the decrypted entry tables; no name, path or listing is written.\n\n",
    );
    report.push_str("| archive | entries | nested | leaf | cipher | names | refused |\n");
    report.push_str("|---|--:|--:|--:|--:|--:|--:|\n");

    // Flagged bodies are decoded only when the snapshot carries content keys; without them every
    // flagged entry would otherwise be counted "would not decode" with no hint why.
    let decode = keys.has_content_keys();

    let mut total = CensusCounts::default();
    for path in &archives {
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut per = CensusCounts::default();
        match Gta6Mount::open_root(&mut file, keys) {
            Ok(mut mount) => {
                mount.archive.resolve_names(resolver);
                census_mount(
                    &mut file, &mount, top_only, decode, keys, resolver, 0, &mut per,
                );
            }
            Err(e) => {
                eprintln!("  {}: {e}", path.display());
                total.refused += 1;
                continue;
            }
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        report.push_str(&format!(
            "| {name} | {} | {} | {} | {} | {} | {} |\n",
            per.entries, per.nested, per.leaf, per.cipher, per.resolved, per.refused
        ));
        total.entries += per.entries;
        total.nested += per.nested;
        total.leaf += per.leaf;
        total.cipher += per.cipher;
        total.resolved += per.resolved;
        total.refused += per.refused;
        total.heads.add(&per.heads);
    }
    report.push_str(&format!(
        "| **total ({} top-level + nested)** | {} | {} | {} | {} | {} | {} |\n",
        archives.len(),
        total.entries,
        total.nested,
        total.leaf,
        total.cipher,
        total.resolved,
        total.refused,
    ));

    let h = &total.heads;
    let scope = if top_only {
        "top-level archives only"
    } else {
        "all archives, nested included"
    };
    report.push_str(&format!(
        "\n## Flagged (content-ciphered) bodies by inflated head ({scope})\n\n"
    ));
    if !decode {
        report.push_str(
            "_The key snapshot carries no content keys, so flagged bodies were not decoded._\n\n",
        );
    }
    report.push_str("| head | count |\n|---|--:|\n");
    report.push_str(&format!("| PSIN (PSO) | {} |\n", h.psin));
    report.push_str(&format!("| RBF0 | {} |\n", h.rbf0));
    report.push_str(&format!("| text / XML | {} |\n", h.text));
    report.push_str(&format!(
        "| headerless (script programs) | {} |\n",
        h.headerless
    ));
    report.push_str(&format!("| other | {} |\n", h.other));
    report.push_str(&format!("| would not decode | {} |\n", h.unreadable));

    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let file = out.join("rpf9-census.md");
    std::fs::write(&file, &report).map_err(|e| format!("{}: {e}", file.display()))?;

    Ok(format!(
        "{} top-level archives, {} nested, {} entries ({} cipher, {} named, {} refused); flagged \
         heads PSIN {} RBF0 {} text {} headerless {} other {} unreadable {}; wrote {}",
        archives.len(),
        total.nested,
        total.entries,
        total.cipher,
        total.resolved,
        total.refused,
        h.psin,
        h.rbf0,
        h.text,
        h.headerless,
        h.other,
        h.unreadable,
        file.display(),
    ))
}

/// Count one mount's entries and, unless `top_only`, recurse into every nested archive it holds. A
/// nested table that cannot be probed or opened, or that would nest past [`CENSUS_MAX_DEPTH`], is
/// counted as a refusal rather than silently dropped.
#[allow(clippy::too_many_arguments)]
fn census_mount(
    file: &mut File,
    mount: &Gta6Mount,
    top_only: bool,
    decode: bool,
    keys: &GtaViKeys,
    resolver: &NameResolver,
    depth: usize,
    acc: &mut CensusCounts,
) {
    for entry in &mount.archive.entries {
        acc.entries += 1;
        if let Some(cipher) = entry.cipher {
            acc.cipher += 1;
            // With content keys, decrypt and inflate the flagged body and classify its head; a body
            // that will not decode is counted as unreadable rather than dropped. Without them the
            // body is left unclassified (the report says the keys were absent).
            if cipher.cipher_type == 17 && decode {
                match mount.read_entry_decrypted(&mut *file, entry, keys) {
                    Ok(body) => acc.heads.classify(&body),
                    Err(_) => acc.heads.unreadable += 1,
                }
            }
        }
        if entry.name.is_some() {
            acc.resolved += 1;
        }
        match mount.entry_begins_archive(file, entry) {
            Ok(true) => {
                acc.nested += 1;
                if top_only {
                    continue;
                }
                if depth >= CENSUS_MAX_DEPTH {
                    acc.refused += 1;
                    continue;
                }
                match mount.open_nested(file, entry, keys) {
                    Ok(mut child) => {
                        child.archive.resolve_names(resolver);
                        census_mount(
                            file,
                            &child,
                            top_only,
                            decode,
                            keys,
                            resolver,
                            depth + 1,
                            acc,
                        );
                    }
                    Err(_) => acc.refused += 1,
                }
            }
            Ok(false) => acc.leaf += 1,
            Err(_) => acc.refused += 1,
        }
    }
}

// --- shared helpers ---------------------------------------------------------

/// Split an inner-path argument into components on either slash. Empty components are dropped, so a
/// combined `nested.rpf/deeper.rpf` and separate arguments reach the same list.
fn split_inner(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Every `*.rpf` directly under `root` and every `pack/*/*.rpf`, sorted — the build's archive set,
/// matching the explorer test's walk.
fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                    out.push(path);
                }
            }
        }
    };
    rpfs_in(root);
    if let Ok(packs) = std::fs::read_dir(root.join("pack")) {
        for pack in packs.flatten() {
            if pack.path().is_dir() {
                rpfs_in(&pack.path());
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
#[path = "rpf9_tests.rs"]
mod tests;
