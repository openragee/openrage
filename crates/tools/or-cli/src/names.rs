//! `openrage-cli hash` and `openrage-cli names`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use or_io::names::{HashFlavour, NameIndex};

use super::census_cap::{Cap, CapUnit, UNLIMITED};

use or_title::install::crawl;

use super::keys::resolve_crypto;
use super::search::resolve_game_root;
use super::usage_error;

pub fn run_hash(args: &[String]) -> ExitCode {
    let mut names: Vec<String> = Vec::new();
    let mut radix = Radix::Hex;
    let mut flavours: Vec<HashFlavour> = HashFlavour::ALL.to_vec();
    let mut json = false;
    let mut from_stdin = false;
    let mut file: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "-h" | "--help" | "help" => {
                print_hash_usage();
                return ExitCode::SUCCESS;
            }
            "--json" => json = true,
            "--stdin" => from_stdin = true,
            _ => {
                let (key, value) = match arg.split_once('=') {
                    Some((k, v)) => (k, Some(v.to_string())),
                    None => (arg, None),
                };
                match key {
                    "--radix" | "--algo" | "--file" => {
                        let v = match value {
                            Some(v) => v,
                            None => {
                                i += 1;
                                match args.get(i) {
                                    Some(v) => v.clone(),
                                    None => return usage_error(&format!("{key} needs a value")),
                                }
                            }
                        };
                        match key {
                            "--radix" => match Radix::parse(&v) {
                                Some(r) => radix = r,
                                None => return usage_error("--radix must be hex or dec"),
                            },
                            "--algo" => match parse_flavours(&v) {
                                Some(f) => flavours = f,
                                None => {
                                    return usage_error("--algo must be folding, literal or both")
                                }
                            },
                            _ => file = Some(PathBuf::from(v)),
                        }
                    }
                    other if other.starts_with("--") => {
                        return usage_error(&format!("unknown option: {other}"))
                    }
                    _ => names.push(arg.to_string()),
                }
            }
        }
        i += 1;
    }

    if from_stdin {
        use std::io::BufRead;
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            push_name(&mut names, &line);
        }
    }
    if let Some(path) = &file {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                for line in text.lines() {
                    push_name(&mut names, line);
                }
            }
            Err(e) => {
                eprintln!("Error: read {}: {e}", path.display());
                return ExitCode::from(1);
            }
        }
    }

    if names.is_empty() {
        return usage_error("give at least one name, or --stdin / --file <path>");
    }

    if json {
        let rows: Vec<serde_json::Value> = names
            .iter()
            .map(|name| {
                let mut row = serde_json::Map::new();
                row.insert("name".into(), serde_json::Value::String(name.clone()));
                for flavour in &flavours {
                    row.insert(
                        flavour_key(*flavour).into(),
                        serde_json::Value::String(radix.render(flavour.of(name))),
                    );
                }
                serde_json::Value::Object(row)
            })
            .collect();
        match serde_json::to_string_pretty(&serde_json::json!({ "hashes": rows })) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("Error: serialize: {e}"),
        }
        return ExitCode::SUCCESS;
    }

    for name in &names {
        let rendered: Vec<String> = flavours
            .iter()
            .map(|f| format!("{}={}", flavour_key(*f), radix.render(f.of(name))))
            .collect();
        println!("{name}  {}", rendered.join("  "));
    }
    ExitCode::SUCCESS
}

fn push_name(names: &mut Vec<String>, line: &str) {
    let trimmed = line.trim();
    if !trimmed.is_empty() {
        names.push(trimmed.to_string());
    }
}

#[derive(Clone, Copy)]
enum Radix {
    Hex,
    Dec,
}

impl Radix {
    fn parse(v: &str) -> Option<Self> {
        match v.to_ascii_lowercase().as_str() {
            "hex" | "16" | "0x" => Some(Radix::Hex),
            "dec" | "10" => Some(Radix::Dec),
            _ => None,
        }
    }

    fn render(self, hash: u32) -> String {
        match self {
            Radix::Hex => format!("0x{hash:08X}"),
            Radix::Dec => hash.to_string(),
        }
    }
}

fn parse_flavours(v: &str) -> Option<Vec<HashFlavour>> {
    match v.to_ascii_lowercase().as_str() {
        "both" | "all" => Some(HashFlavour::ALL.to_vec()),
        "folding" | "case-folding" | "string" => Some(vec![HashFlavour::CaseFolding]),
        "literal" | "case-sensitive" => Some(vec![HashFlavour::Literal]),
        _ => None,
    }
}

fn flavour_key(flavour: HashFlavour) -> &'static str {
    match flavour {
        HashFlavour::CaseFolding => "folding",
        HashFlavour::Literal => "literal",
    }
}

/// Accepts what a user would paste: `0xAB34CAAD`, `AB34CAAD`, `2872724653`.
pub(crate) fn parse_hash(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16).ok();
    }
    if trimmed.len() == 8 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return u32::from_str_radix(trimmed, 16).ok();
    }
    trimmed.parse::<u32>().ok()
}

/// How many names an index will hold before it stops.
///
/// ⚠ **A name costs two map entries, not one.** `NameIndex` records every
/// name under both hash flavours so a hash of either kind resolves, so the
/// footprint is twice what the count suggests. A retail install's filenames
/// and path segments come to a few hundred thousand before a single
/// `.nametable` is read; this ceiling is above that and below the point where
/// the run decides how much memory the machine has.
const DEFAULT_MAX_NAMES: u64 = 2_000_000;

const MAX_NAMES_FLAG: &str = "--max-names";

pub fn run_names(args: &[String]) -> ExitCode {
    let mut game: Option<PathBuf> = None;
    let mut keys: Option<PathBuf> = None;
    let mut resolve: Vec<String> = Vec::new();
    let mut tables: Vec<PathBuf> = Vec::new();
    let mut export: Option<PathBuf> = None;
    let mut json = false;
    let mut cap = Cap::new(DEFAULT_MAX_NAMES, CapUnit::Count("name(s)"));

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (arg, None),
        };
        match key {
            "-h" | "--help" | "help" => {
                print_names_usage();
                return ExitCode::SUCCESS;
            }
            "--json" => json = true,
            "--game" | "--keys" | "--resolve" | "--nametable" | "--export" | MAX_NAMES_FLAG => {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        match args.get(i) {
                            Some(v) => v.clone(),
                            None => return usage_error(&format!("{key} needs a value")),
                        }
                    }
                };
                match key {
                    "--game" => game = Some(PathBuf::from(v)),
                    "--keys" => keys = Some(PathBuf::from(v)),
                    "--resolve" => resolve.push(v),
                    "--nametable" => tables.push(PathBuf::from(v)),
                    MAX_NAMES_FLAG => match cap.parse(MAX_NAMES_FLAG, &v) {
                        Ok(c) => cap = c,
                        Err(e) => return usage_error(&e),
                    },
                    _ => export = Some(PathBuf::from(v)),
                }
            }
            other => return usage_error(&format!("unknown option: {other}")),
        }
        i += 1;
    }

    let game_root = match resolve_game_root(game) {
        Ok(p) => p,
        Err(e) => return usage_error(&e),
    };
    if !game_root.is_dir() {
        return usage_error(&format!(
            "game root is not a directory: {}",
            game_root.display()
        ));
    }

    let title = or_title::Game::detect_from_root(&game_root);
    let resolved = resolve_crypto(&game_root, keys.as_deref());
    let mut warnings = resolved.notes;

    let mut index = NameIndex::new(title.asset_slug());
    let mut census = Census::default();
    let mut table_warnings: Vec<String> = Vec::new();
    eprintln!(
        "openrage-cli: indexing names in {} (Ctrl-C stops and still reports) ...",
        game_root.display()
    );

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = Arc::clone(&cancel);
        let _ = super::jobs::install_stop_handler(move || cancel.store(true, Ordering::Relaxed));
    }

    let _ = crawl::crawl(&game_root, resolved.provider, &mut warnings, &mut |found| {
        if cancel.load(Ordering::Relaxed) || cap.reached(index.names() as u64) {
            return crawl::ControlFlow::Break(());
        }
        census.files += 1;
        index.insert_with_stem(found.leaf);
        for segment in found.path.split(['/', '\\']) {
            index.insert(segment);
        }
        if !found.leaf.to_ascii_lowercase().ends_with(".nametable") {
            return crawl::ControlFlow::Continue(());
        }
        census.name_tables += 1;
        match found.read().and_then(|bytes| {
            or_io::gta5::data::nametable::parse(&bytes).map_err(|e| e.to_string())
        }) {
            Ok(table) => {
                census.table_names += table.names.len();
                census.skipped_entries += table.skipped.len();
                for skipped in &table.skipped {
                    table_warnings.push(format!(
                        "{}/{} entry {}: {}",
                        found.container, found.path, skipped.index, skipped.reason
                    ));
                }
                index.extend(table.names);
            }
            Err(e) => table_warnings.push(format!("{}/{}: {e}", found.container, found.path)),
        }
        crawl::ControlFlow::Continue(())
    });
    warnings.append(&mut table_warnings);

    let stopped_short = cap.note(index.names() as u64, MAX_NAMES_FLAG);
    let interrupted = cancel.load(Ordering::Relaxed);

    for path in &tables {
        match ingest_table_path(path, &mut index) {
            Ok(n) => {
                census.name_tables += 1;
                census.table_names += n;
            }
            Err(e) => warnings.push(e),
        }
    }

    let resolutions: Vec<(String, Option<(String, &'static str)>)> = resolve
        .iter()
        .map(|raw| {
            let Some(hash) = parse_hash(raw) else {
                return (raw.clone(), None);
            };
            (
                raw.clone(),
                index
                    .resolve(hash)
                    .map(|r| (r.name.to_string(), r.flavour.label())),
            )
        })
        .collect();

    if let Some(path) = &export {
        // ⚠ A table exported from a short run is a table with names missing,
        // and nothing in the file says which.
        if let Some(note) = &stopped_short {
            eprintln!("note: {note}");
            eprintln!("note: the exported table is therefore incomplete");
        }
        let names = index.sorted_names();
        if let Err(e) = std::fs::write(path, or_io::gta5::data::nametable::write(&names)) {
            eprintln!("Error: write {}: {e}", path.display());
            return ExitCode::from(1);
        }
        eprintln!("wrote {} name(s) to {}", names.len(), path.display());
    }

    if json {
        let (truncated, limit) = cap.json(index.names() as u64);
        let doc = serde_json::json!({
            "title": index.title(),
            "files": census.files,
            "names": index.names(),
            "nameTables": census.name_tables,
            "nameTableNames": census.table_names,
            "nameTableEntriesSkipped": census.skipped_entries,
            "collisions": index.collisions().len(),
            "truncated": truncated,
            "interrupted": interrupted,
            "maxNames": limit,
            "resolved": resolutions.iter().map(|(raw, hit)| serde_json::json!({
                "input": raw,
                "name": hit.as_ref().map(|(n, _)| n.clone()),
                "via": hit.as_ref().map(|(_, f)| f.to_string()),
            })).collect::<Vec<_>>(),
            "warnings": warnings,
        });
        match serde_json::to_string_pretty(&doc) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("Error: serialize: {e}"),
        }
        return ExitCode::SUCCESS;
    }

    for w in &warnings {
        eprintln!("note: {w}");
    }
    println!("title            {}", index.title());
    println!("files walked     {}", census.files);
    println!("distinct names   {}", index.names());
    println!(
        "name tables      {} ({} names, {} entries skipped)",
        census.name_tables, census.table_names, census.skipped_entries
    );
    println!("collisions       {}", index.collisions().len());
    println!(
        "name ceiling     {}",
        if cap.is_unlimited() {
            format!("{UNLIMITED} (asked for)")
        } else {
            format!("{} ({MAX_NAMES_FLAG})", cap.limit().unwrap_or(0))
        }
    );
    for collision in index.collisions().iter().take(10) {
        println!(
            "  0x{:08X} [{}] kept {}, dropped {}",
            collision.hash,
            collision.flavour.label(),
            collision.kept,
            collision.dropped
        );
    }
    if interrupted {
        println!("INTERRUPTED: stopped on Ctrl-C, so the counts above are partial.");
    }
    if let Some(note) = &stopped_short {
        println!("{note}");
    }
    for (raw, hit) in &resolutions {
        match hit {
            Some((name, via)) => println!("{raw} -> {name}  [{via}]"),
            None => println!("{raw} -> (unresolved)"),
        }
    }
    ExitCode::SUCCESS
}

#[derive(Default)]
struct Census {
    files: usize,
    name_tables: usize,
    table_names: usize,
    skipped_entries: usize,
}

fn ingest_table_path(path: &Path, index: &mut NameIndex) -> Result<usize, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let table = or_io::gta5::data::nametable::parse(&bytes)
        .map_err(|e| format!("parse {}: {e}", path.display()))?;
    let count = table.names.len();
    index.extend(table.names);
    Ok(count)
}

fn print_hash_usage() {
    eprintln!("Hash names the way RAGE does.");
    eprintln!();
    eprintln!("Usage: openrage-cli hash [name...] [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --stdin          Read names from stdin, one per line.");
    eprintln!("  --file <path>    Read names from a file, one per line.");
    eprintln!("  --algo=<a>       folding (atStringHash), literal, or both. Default both.");
    eprintln!("  --radix=<r>      hex or dec. Default hex.");
    eprintln!("  --json           Machine-readable output.");
    eprintln!();
    eprintln!("The two differ: `folding` lowercases and turns \\ into /, and is what asset");
    eprintln!("and audio names use; `literal` is what PSO metadata stores. A hash quoted");
    eprintln!("without saying which one it is cannot be checked.");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  openrage-cli hash BIFTA");
    eprintln!("  openrage-cli hash --file names.txt --algo=folding --radix=dec");
}

fn print_names_usage() {
    eprintln!("Build a hash -> name index over an install and query it.");
    eprintln!();
    eprintln!("Usage: openrage-cli names [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --game <dir>       GTA install root. Default: the app's selected game.");
    eprintln!("  --keys <file>      Override the keys cache (normally auto-resolved).");
    eprintln!("  --resolve <hash>   Look a hash up. Repeatable. 0xAB34CAAD, AB34CAAD or");
    eprintln!("                     decimal. Says which hash function matched.");
    eprintln!("  --nametable <file> Ingest an extra .nametable beside the install's own.");
    eprintln!("  --export <file>    Write every indexed name out as a .nametable.");
    eprintln!("  --json             Machine-readable output.");
    eprintln!(
        "  --max-names <n>    Stop after this many distinct names. Default {DEFAULT_MAX_NAMES}."
    );
    eprintln!("                     `--max-names={UNLIMITED}` lifts it; a whole install can");
    eprintln!("                     hold more names than the machine has memory for.");
    eprintln!();
    eprintln!("The index is per title and never answers with another title's name.");
    eprintln!("A run that hits the ceiling, or that you stop with Ctrl-C, says so in its");
    eprintln!("output — the counts are then partial and must not be quoted as a figure");
    eprintln!("over the install.");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  openrage-cli names --game \"C:/Games/GTAV\"");
    eprintln!("  openrage-cli names --resolve 0xEB298297 --resolve 0x05852838");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠ **An index over a whole install is the output, so the only thing
    /// that bounds it is a ceiling**; a default that drifts back to unlimited
    /// is the failure this pins.
    #[test]
    fn the_name_ceiling_is_finite_by_default_and_lifts_only_when_spelled() {
        let cap = Cap::new(DEFAULT_MAX_NAMES, CapUnit::Count("name(s)"));
        assert_eq!(cap.limit(), Some(DEFAULT_MAX_NAMES));
        assert!(!cap.is_unlimited());

        assert!(cap
            .parse(MAX_NAMES_FLAG, UNLIMITED)
            .expect("the word parses")
            .is_unlimited());
        let why = cap
            .parse(MAX_NAMES_FLAG, "0")
            .expect_err("zero is not a ceiling");
        assert!(why.contains(UNLIMITED), "{why}");
    }

    #[test]
    fn a_hash_is_accepted_in_every_spelling_a_user_would_paste() {
        assert_eq!(parse_hash("0xEB298297"), Some(0xEB29_8297));
        assert_eq!(parse_hash("0XEB298297"), Some(0xEB29_8297));
        assert_eq!(parse_hash("EB298297"), Some(0xEB29_8297));
        assert_eq!(parse_hash("  0xEB298297 "), Some(0xEB29_8297));
        assert_eq!(parse_hash("3945366167"), Some(0xEB29_8297));
        assert_eq!(parse_hash(""), None);
        assert_eq!(parse_hash("nonsense"), None);
        assert_eq!(parse_hash("0xFFFFFFFFF"), None);
    }

    /// ⚠ Eight decimal digits are ambiguous and hex wins, because a hash is
    /// quoted in hex everywhere in this project and in every tool beside it.
    #[test]
    fn an_eight_digit_number_reads_as_hex() {
        assert_eq!(parse_hash("12345678"), Some(0x1234_5678));
        assert_eq!(parse_hash("123456789"), Some(123_456_789));
        assert_eq!(parse_hash("1234567"), Some(1_234_567));
    }

    #[test]
    fn a_radix_renders_the_same_number_two_ways() {
        assert_eq!(Radix::Hex.render(0xEB29_8297), "0xEB298297");
        assert_eq!(Radix::Dec.render(0xEB29_8297), "3945366167");
        assert!(Radix::parse("HEX").is_some());
        assert!(Radix::parse("octal").is_none());
    }

    #[test]
    fn an_algo_name_selects_the_function_it_names() {
        assert_eq!(parse_flavours("both").unwrap().len(), 2);
        assert_eq!(
            parse_flavours("folding").unwrap(),
            vec![HashFlavour::CaseFolding]
        );
        assert_eq!(
            parse_flavours("literal").unwrap(),
            vec![HashFlavour::Literal]
        );
        assert!(parse_flavours("md5").is_none());
    }

    #[test]
    fn ingesting_a_name_table_makes_its_names_resolvable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audio.nametable");
        std::fs::write(&path, b"BIFTA\0KALAHARI\0").unwrap();

        let mut index = NameIndex::new("gta5");
        assert_eq!(ingest_table_path(&path, &mut index).unwrap(), 2);
        assert_eq!(index.display(0xEB29_8297), "BIFTA");
        assert_eq!(index.display(0x0585_2838), "KALAHARI");

        let missing = dir.path().join("nope.nametable");
        assert!(ingest_table_path(&missing, &mut index).is_err());

        let bad = dir.path().join("bad.nametable");
        std::fs::write(&bad, [0x97u8, 0x82, 0x29, 0xEB]).unwrap();
        assert!(ingest_table_path(&bad, &mut index).is_err());
    }
}
