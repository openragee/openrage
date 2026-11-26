use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use or_io::awc::AwcKey;
use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_title::{
    build_mounted_game, build_mounted_game_with, DlcCutoff, GtaInstall, ListedPackState, MapMode,
    MountOptions, MountedGame, SetupContentType,
};

use or_title::install::crawl;

use or_title::install::content;

use super::census_cap::{Cap, CapUnit, UNLIMITED};
use super::keys::resolve_crypto;
use super::registry;
use super::usage_error;
use super::verify;

const BUILD_TO_PACK: &[(u32, &str)] = &[
    (1604, "mpchristmas2018"),
    (1737, "mpvinewood"),
    (1868, "mpheist3"),
    (2060, "mpsum"),
    (2189, "mpheist4"),
    (2372, "mptuner"),
    (2545, "mpsecurity"),
    (2612, "mpg9ec"),
    (2699, "mpsum2"),
    (2802, "mpchristmas3"),
    (2944, "mp2023_01"),
    (3095, "mp2023_02"),
    (3258, "mp2024_01"),
    (3407, "mp2024_02"),
    (3570, "mp2025_01"),
    (3751, "mp2025_02"),
];

struct Args {
    pattern: Option<String>,
    game: Option<PathBuf>,
    keys: Option<PathBuf>,
    mounted: bool,
    dlc: Option<String>,
    file_types: Vec<String>,
    ext: Vec<String>,
    device: Option<String>,
    count_only: bool,
    json: bool,
    text: Option<String>,
    hex: Option<String>,
    utf16: bool,
    case_sensitive: bool,
    under: Option<String>,
    max_hits: Cap,
    max_file_bytes: usize,
    preview: bool,
    verify: bool,
}

/// A content search reads every file it is not filtered out of, so the
/// default ceiling keeps one oversized entry from deciding how much memory
/// the run takes.
const DEFAULT_MAX_FILE_BYTES: usize = 64 << 20;

/// How many hits a run will hold before it stops.
///
/// ⚠ **A name search with no pattern matches every file in the install** —
/// around 170,000 of them, each `Hit` four strings. That is tens of megabytes
/// rather than gigabytes, so this ceiling was not the 38.4 GB OOM; it caps an
/// *unlimited by default* shape. A query wanting more than this wants `--json`
/// and a file.
pub(crate) const DEFAULT_MAX_HITS: u64 = 100_000;

const MAX_HITS_FLAG: &str = "--max-hits";

#[derive(Default)]
pub(crate) struct HitFilter {
    pub(crate) ext: Vec<String>,
    pub(crate) device: Option<String>,
}

impl HitFilter {
    pub(crate) fn is_empty(&self) -> bool {
        self.ext.is_empty() && self.device.is_none()
    }

    fn keeps(&self, hit: &Hit) -> bool {
        if !self.ext.is_empty() {
            let leaf = leaf_of(&hit.path);
            let ext = leaf.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
            if !self.ext.iter().any(|w| w.eq_ignore_ascii_case(ext)) {
                return false;
            }
        }
        if let Some(dev) = &self.device {
            let device = hit.path.split(':').next().unwrap_or("");
            if !device.to_ascii_lowercase().contains(dev.as_str()) {
                return false;
            }
        }
        true
    }
}

pub(crate) fn apply_filter(hits: &mut Vec<Hit>, filter: &HitFilter) {
    if !filter.is_empty() {
        hits.retain(|h| filter.keeps(h));
    }
}

pub(crate) fn split_exts(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Hit {
    pub(crate) path: String,
    pub(crate) container: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) file_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pack: Option<String>,
}

pub fn run(args: &[String]) -> ExitCode {
    let parsed = match parse_args(args) {
        Ok(Some(p)) => p,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            eprintln!();
            print_usage();
            return ExitCode::from(2);
        }
    };

    if let Err(e) = validate(&parsed) {
        eprintln!("Error: {e}");
        return ExitCode::from(2);
    }

    let game_root = match resolve_game_root(parsed.game.clone()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(2);
        }
    };
    if !game_root.is_dir() {
        eprintln!(
            "Error: game root is not a directory: {}",
            game_root.display()
        );
        return ExitCode::from(2);
    }

    let resolved = resolve_crypto(&game_root, parsed.keys.as_deref());
    let provider = resolved.provider;
    let mut warnings = resolved.notes;

    if parsed.verify {
        return run_verify(&game_root, provider, resolved.awc_key, &parsed, warnings);
    }
    if parsed.text.is_some() || parsed.hex.is_some() {
        return run_content(&game_root, provider, &parsed, warnings);
    }

    let matcher = Matcher::new(parsed.pattern.clone());

    let hits = if parsed.mounted {
        search_mounted(&game_root, provider, &parsed, &matcher, &mut warnings)
    } else {
        search_raw(
            &game_root,
            provider,
            &matcher,
            parsed.max_hits,
            &mut warnings,
        )
    };

    let mut hits = match hits {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(1);
        }
    };

    // ⚠ Taken before the filter: the ceiling is on what the run *held*, and
    // filtering can drop a truncated list below its own limit, at which point
    // the report would fall silent about having stopped short.
    let stopped_short = parsed.max_hits.note(hits.len() as u64, MAX_HITS_FLAG);

    let filter = HitFilter {
        ext: parsed.ext.clone(),
        device: parsed.device.clone(),
    };
    apply_filter(&mut hits, &filter);

    match (parsed.count_only, parsed.json) {
        (true, true) => print_count_json(hits.len(), &warnings),
        (true, false) => {
            for w in &warnings {
                eprintln!("note: {w}");
            }
            println!("{}", hits.len());
        }
        (false, true) => print_json(&hits, &warnings),
        (false, false) => print_text(&hits, &warnings),
    }
    if let Some(note) = &stopped_short {
        if parsed.json {
            eprintln!("note: {note}");
        } else {
            println!("{note}");
        }
    }
    ExitCode::SUCCESS
}

/// ⚠ Hits are printed as they arrive rather than collected. A whole-install
/// content search is minutes of work, and a run that can only report at the
/// end is indistinguishable from a hang and cannot be interrupted usefully.
fn run_content(
    game_root: &Path,
    provider: Arc<dyn RpfCryptoProvider>,
    args: &Args,
    mut warnings: Vec<String>,
) -> ExitCode {
    let needle = match (&args.text, &args.hex) {
        (Some(text), _) if args.utf16 => or_io::Needle::utf16(text, args.case_sensitive),
        (Some(text), _) => or_io::Needle::text(text, args.case_sensitive),
        (_, Some(hex)) => match or_io::Needle::hex(hex) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("Error: --hex: {e}");
                return ExitCode::from(2);
            }
        },
        _ => unreachable!("validate() requires one of them"),
    };

    let scope = match &args.under {
        Some(prefix) => content::Scope::Under(prefix.replace('\\', "/")),
        None => content::Scope::Everywhere,
    };
    let query = content::Query {
        needle: &needle,
        scope: &scope,
        extensions: &args.ext,
        max_file_bytes: args.max_file_bytes,
        hits_per_file: 64,
        preview_bytes: if args.preview && !args.count_only && !args.json {
            64
        } else {
            0
        },
        max_hits: args.max_hits.limit().unwrap_or(0) as usize,
    };

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = Arc::clone(&cancel);
        let _ = super::jobs::install_stop_handler(move || cancel.store(true, Ordering::Relaxed));
    }

    for w in &warnings {
        eprintln!("note: {w}");
    }

    let mut collected: Vec<content::ContentHit> = Vec::new();
    let progress = content::search_contents(
        game_root,
        provider,
        &query,
        &cancel,
        &mut warnings,
        &mut |hit, progress| {
            if args.json {
                collected.push(hit.clone());
                return;
            }
            if args.count_only {
                return;
            }
            let offsets: Vec<String> = hit.offsets.iter().map(|o| format!("0x{o:X}")).collect();
            println!(
                "{}  <= {}  [{} hit(s) at {}]",
                hit.path,
                hit.container,
                hit.offsets.len(),
                offsets.join(", ")
            );
            if let Some(preview) = &hit.preview {
                for line in preview.lines() {
                    println!("    {line}");
                }
            }
            if progress.hits % 64 == 0 {
                eprintln!(
                    "  … {} file(s) searched, {} match(es)",
                    progress.files_read, progress.hits
                );
            }
        },
    );

    if args.json {
        let doc = serde_json::json!({
            "hits": collected,
            "progress": progress,
            "warnings": warnings,
        });
        match serde_json::to_string_pretty(&doc) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("Error: serialize: {e}"),
        }
        return ExitCode::SUCCESS;
    }

    if args.count_only {
        println!("{}", progress.hits);
    } else {
        println!(
            "\n{} match(es) in {} file(s) read ({} seen, {} skipped){}",
            progress.hits,
            progress.files_read,
            progress.files_seen,
            progress.unreadable,
            if progress.stopped { ", stopped" } else { "" }
        );
    }
    ExitCode::SUCCESS
}

/// Parse every file the readers claim, and report what refused.
fn run_verify(
    game_root: &Path,
    provider: Arc<dyn RpfCryptoProvider>,
    awc_key: AwcKey,
    args: &Args,
    mut warnings: Vec<String>,
) -> ExitCode {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = Arc::clone(&cancel);
        let _ = super::jobs::install_stop_handler(move || cancel.store(true, Ordering::Relaxed));
    }
    for w in &warnings {
        eprintln!("note: {w}");
    }
    eprintln!(
        "openrage-cli: parsing every readable file in {} ... (Ctrl-C stops and still reports)",
        game_root.display()
    );

    let sweep = verify::sweep(
        game_root,
        provider,
        awc_key,
        &args.ext,
        &cancel,
        &mut warnings,
        &mut |sweep| {
            eprintln!("  … {} parsed, {} refused", sweep.attempted, sweep.refused);
        },
    );

    if args.json {
        let doc = serde_json::json!({
            "seen": sweep.seen,
            "attempted": sweep.attempted,
            "refused": sweep.refused,
            "stopped": sweep.stopped,
            "byExtension": sweep.by_extension.iter().map(|(ext, (a, r))| {
                serde_json::json!({ "ext": ext, "attempted": a, "refused": r })
            }).collect::<Vec<_>>(),
            "reasons": sweep.reasons.values().map(|(count, example)| {
                serde_json::json!({ "count": count, "example": example })
            }).collect::<Vec<_>>(),
            "warnings": warnings,
        });
        match serde_json::to_string_pretty(&doc) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("Error: serialize: {e}"),
        }
    } else {
        print!("{}", verify::report(&sweep, 25));
    }
    ExitCode::SUCCESS
}

fn parse_args(args: &[String]) -> Result<Option<Args>, String> {
    let mut out = Args {
        pattern: None,
        game: None,
        keys: None,
        mounted: false,
        dlc: None,
        file_types: Vec::new(),
        ext: Vec::new(),
        device: None,
        count_only: false,
        json: false,
        text: None,
        hex: None,
        utf16: false,
        case_sensitive: false,
        under: None,
        max_hits: Cap::new(DEFAULT_MAX_HITS, CapUnit::Count("hit(s)")),
        max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        preview: true,
        verify: false,
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print_usage();
                return Ok(None);
            }
            "--mounted" => out.mounted = true,
            "--json" => out.json = true,
            "--utf16" => out.utf16 = true,
            "--case-sensitive" | "--caseSensitive" => out.case_sensitive = true,
            "--no-preview" => out.preview = false,
            "--verify" => out.verify = true,
            "--text" => out.text = Some(next_value(args, &mut i, "--text")?),
            "--hex" => out.hex = Some(next_value(args, &mut i, "--hex")?),
            "--under" => out.under = Some(next_value(args, &mut i, "--under")?),
            MAX_HITS_FLAG => {
                let v = next_value(args, &mut i, MAX_HITS_FLAG)?;
                out.max_hits = out.max_hits.parse(MAX_HITS_FLAG, &v)?;
            }
            "--max-file-size" => {
                out.max_file_bytes = parse_usize(&next_value(args, &mut i, "--max-file-size")?)?
            }
            "--game" => {
                out.game = Some(PathBuf::from(next_value(args, &mut i, "--game")?));
            }
            "--keys" => {
                out.keys = Some(PathBuf::from(next_value(args, &mut i, "--keys")?));
            }
            "--dlc" => {
                out.dlc = Some(next_value(args, &mut i, "--dlc")?);
            }
            "--fileTypes" | "--filetypes" | "--file-types" => {
                let v = next_value(args, &mut i, "--fileTypes")?;
                out.file_types = split_file_types(&v);
            }
            "--ext" => {
                let v = next_value(args, &mut i, "--ext")?;
                out.ext = split_exts(&v);
            }
            "--device" => {
                out.device = Some(next_value(args, &mut i, "--device")?.to_ascii_lowercase());
            }
            "--count-only" | "--countOnly" | "--count" => out.count_only = true,
            other => {
                if let Some(v) = other.strip_prefix("--game=") {
                    out.game = Some(PathBuf::from(v));
                } else if let Some(v) = other.strip_prefix("--keys=") {
                    out.keys = Some(PathBuf::from(v));
                } else if let Some(v) = other.strip_prefix("--dlc=") {
                    out.dlc = Some(v.to_string());
                } else if let Some(v) = other
                    .strip_prefix("--fileTypes=")
                    .or_else(|| other.strip_prefix("--filetypes="))
                    .or_else(|| other.strip_prefix("--file-types="))
                {
                    out.file_types = split_file_types(v);
                } else if let Some(v) = other.strip_prefix("--ext=") {
                    out.ext = split_exts(v);
                } else if let Some(v) = other.strip_prefix("--device=") {
                    out.device = Some(v.to_ascii_lowercase());
                } else if let Some(v) = other.strip_prefix("--text=") {
                    out.text = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--hex=") {
                    out.hex = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--under=") {
                    out.under = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--max-hits=") {
                    out.max_hits = out.max_hits.parse(MAX_HITS_FLAG, v)?;
                } else if let Some(v) = other.strip_prefix("--max-file-size=") {
                    out.max_file_bytes = parse_usize(v)?;
                } else if other.starts_with("--") {
                    return Err(format!("unknown option: {other}"));
                } else if out.pattern.is_none() {
                    out.pattern = Some(other.to_string());
                } else {
                    return Err(format!("unexpected extra argument: {other}"));
                }
            }
        }
        i += 1;
    }
    Ok(Some(out))
}

/// Accepts `4096`, `512K`, `64M` and `2G`.
fn parse_usize(raw: &str) -> Result<usize, String> {
    let trimmed = raw.trim();
    let (digits, scale) = match trimmed.chars().last() {
        Some('K') | Some('k') => (&trimmed[..trimmed.len() - 1], 1usize << 10),
        Some('M') | Some('m') => (&trimmed[..trimmed.len() - 1], 1usize << 20),
        Some('G') | Some('g') => (&trimmed[..trimmed.len() - 1], 1usize << 30),
        _ => (trimmed, 1),
    };
    digits
        .parse::<usize>()
        .map_err(|_| format!("`{raw}` is not a size"))?
        .checked_mul(scale)
        .ok_or_else(|| format!("`{raw}` does not fit"))
}

fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn split_file_types(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn validate(args: &Args) -> Result<(), String> {
    if args.verify {
        if args.text.is_some() || args.hex.is_some() || args.pattern.is_some() {
            return Err("--verify parses every file and takes no needle".into());
        }
        if args.mounted {
            return Err("--verify is a raw crawl; --mounted lists rather than opens".into());
        }
        return Ok(());
    }
    let content = args.text.is_some() || args.hex.is_some();
    if content {
        if args.text.is_some() && args.hex.is_some() {
            return Err("--text and --hex are two different needles; give one".into());
        }
        if args.mounted {
            return Err(
                "--mounted lists what the engine would load and does not open it; a content \
                 search is a raw crawl"
                    .into(),
            );
        }
        if args.pattern.is_some() {
            return Err(
                "a name pattern and --text/--hex are two searches; filter names with --ext \
                 and --under instead"
                    .into(),
            );
        }
        if args.hex.is_some() && (args.utf16 || args.case_sensitive) {
            return Err("--utf16 and --case-sensitive describe text, not --hex".into());
        }
        return Ok(());
    }

    if args.utf16 || args.case_sensitive || args.under.is_some() {
        return Err("--utf16, --case-sensitive and --under apply to --text/--hex".into());
    }
    if !args.mounted {
        if args.dlc.is_some() {
            return Err("--dlc is only valid with --mounted".into());
        }
        if !args.file_types.is_empty() {
            return Err("--fileTypes is only valid with --mounted".into());
        }
        if args.device.is_some() {
            return Err("--device is only valid with --mounted (raw paths have no device)".into());
        }
    }
    if args.pattern.is_none() && args.file_types.is_empty() && args.ext.is_empty() {
        return Err("provide a name pattern, --ext, or --fileTypes with --mounted".into());
    }
    Ok(())
}

pub(crate) fn resolve_game_root(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    let prefs_path = or_title::config::config_dir()
        .map(|dir| dir.join("prefs.json"))
        .ok_or_else(|| "no config directory; pass --game <dir>".to_string())?;
    let bytes = std::fs::read(&prefs_path).map_err(|_| {
        format!(
            "no --game given and no prefs at {} (open a game in the app first, or pass --game)",
            prefs_path.display()
        )
    })?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse prefs.json: {e}"))?;
    let active = v
        .get("active_game")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "no active game in prefs; pass --game <dir>".to_string())?;
    let game: or_title::Game =
        serde_json::from_value(serde_json::Value::String(active.to_string()))
            .map_err(|_| format!("unknown active game '{active}'; pass --game <dir>"))?;
    let paths = v.get("game_paths");
    let path = paths
        .and_then(|g| g.get(game.asset_slug()))
        .or_else(|| paths.and_then(|g| g.get(game.asset_slug().replace('-', "_"))))
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("active game '{active}' has no path in prefs; pass --game <dir>"))?;
    Ok(PathBuf::from(path))
}

pub(crate) fn resolve_cutoff(dlc: Option<&str>) -> DlcCutoff {
    let Some(raw) = dlc else {
        return DlcCutoff::All;
    };
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("basegame") || trimmed == "0" {
        return DlcCutoff::BaseGame;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(build) = trimmed.parse::<u32>() {
            let mut chosen: Option<&str> = None;
            for (b, name) in BUILD_TO_PACK {
                if *b <= build {
                    chosen = Some(name);
                }
            }
            return match chosen {
                Some(name) => DlcCutoff::NameHash(name.to_string()),
                None => DlcCutoff::BaseGame,
            };
        }
    }
    DlcCutoff::NameHash(trimmed.to_string())
}

fn search_mounted(
    game_root: &Path,
    provider: Arc<dyn RpfCryptoProvider>,
    args: &Args,
    matcher: &Matcher,
    warnings: &mut Vec<String>,
) -> Result<Vec<Hit>, String> {
    let install = GtaInstall::from_path(game_root)
        .ok_or_else(|| format!("invalid install root: {}", game_root.display()))?;
    let cutoff = resolve_cutoff(args.dlc.as_deref());
    let game =
        build_mounted_game(&install, provider, cutoff).map_err(|e| format!("mount failed: {e}"))?;
    warnings.extend(game.diagnostics.messages().iter().map(|d| d.to_string()));

    let (hits, query_warnings) = query_mounted(&game, &args.file_types, matcher, args.max_hits);
    warnings.extend(query_warnings);
    Ok(hits)
}

pub(crate) fn query_mounted(
    game: &MountedGame,
    file_types: &[String],
    matcher: &Matcher,
    cap: Cap,
) -> (Vec<Hit>, Vec<String>) {
    let mut hits = Vec::new();
    let mut warnings = Vec::new();

    if !file_types.is_empty() {
        let want = |ft: &str| file_types.iter().any(|t| t.eq_ignore_ascii_case(ft));

        let locate = |filename: &str| -> Option<String> {
            game.vfs.locate(&registry::expand_platform(filename))
        };
        let mut unresolved = 0usize;

        let base = registry::load_base_game(&game.vfs);
        warnings.extend(base.warnings);
        for df in &base.files {
            if cap.reached(hits.len() as u64) {
                break;
            }
            if want(&df.file_type) && matcher.matches(leaf_of(&df.filename)) {
                let Some(container) = locate(&df.filename) else {
                    unresolved += 1;
                    continue;
                };
                hits.push(Hit {
                    path: df.filename.clone(),
                    container,
                    file_type: Some(df.file_type.clone()),
                    disabled: Some(df.disabled),
                    pack: Some("basegame".into()),
                });
            }
        }

        for pack in &game.packs {
            for df in &pack.data_files {
                if cap.reached(hits.len() as u64) {
                    break;
                }
                if !want(&df.file_type) {
                    continue;
                }
                if !matcher.matches(leaf_of(&df.filename)) {
                    continue;
                }
                let Some(container) = locate(&df.filename) else {
                    unresolved += 1;
                    continue;
                };
                hits.push(Hit {
                    path: df.filename.clone(),
                    container,
                    file_type: Some(df.file_type.clone()),
                    disabled: Some(df.disabled),
                    pack: Some(pack.name_hash.clone()),
                });
            }
        }

        if unresolved > 0 {
            warnings.push(format!(
                "skipped {unresolved} registry entr{} with no shipped file (dangling content.xml reference)",
                if unresolved == 1 { "y" } else { "ies" }
            ));
        }
    } else {
        game.for_each_file(|logical, container, leaf| {
            if cap.reached(hits.len() as u64) {
                return;
            }
            if matcher.matches(leaf) {
                hits.push(Hit {
                    path: logical.to_string(),
                    container: container.to_string(),
                    file_type: None,
                    disabled: None,
                    pack: None,
                });
            }
        });
    }

    (hits, warnings)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackInfo {
    pub(crate) name_hash: String,
    pub(crate) device_name: String,
    pub(crate) content_type: String,
    pub(crate) order: i32,
    pub(crate) minor_order: i32,
    pub(crate) is_level_pack: bool,
    pub(crate) sub_pack_count: u32,
    pub(crate) data_files: usize,
    pub(crate) source: String,
}

fn content_type_label(t: SetupContentType) -> &'static str {
    match t {
        SetupContentType::Unknown => "unknown",
        SetupContentType::TitleUpdate => "title-update",
        SetupContentType::LevelPack => "level",
        SetupContentType::CompatPack => "compat",
        SetupContentType::PaidPack => "paid",
    }
}

pub(crate) fn pack_infos(game: &MountedGame) -> Vec<PackInfo> {
    game.packs
        .iter()
        .map(|p| PackInfo {
            name_hash: p.name_hash.clone(),
            device_name: p.device_name.clone(),
            content_type: content_type_label(p.setup.content_type).to_string(),
            order: p.setup.order,
            minor_order: p.setup.minor_order,
            is_level_pack: p.setup.is_level_pack,
            sub_pack_count: p.setup.sub_pack_count,
            data_files: p.data_files.len(),
            source: p.source_path.display().to_string(),
        })
        .collect()
}

/// Where a `dlc-list` mount stops and which mode it takes; `--dlc` and `--level` both say where to stop, so one is given.
pub(crate) fn dlc_list_mount(
    dlc: Option<&str>,
    level: Option<&str>,
    mode: Option<&str>,
) -> Result<(DlcCutoff, MountOptions), String> {
    let cutoff = match (dlc, level) {
        (Some(_), Some(_)) => {
            return Err("--dlc and --level both say where to stop; give one".into())
        }
        (_, Some(level)) => DlcCutoff::Level(level.trim().to_string()),
        (dlc, None) => resolve_cutoff(dlc),
    };
    let mode = match mode {
        Some(word) => Some(
            MapMode::from_label(word)
                .ok_or_else(|| format!("--mode is story or multiplayer, not `{word}`"))?,
        ),
        None => None,
    };
    let options = MountOptions {
        mode,
        ..MountOptions::default()
    };
    Ok((cutoff, options))
}

/// The listed packs a mount left out, in the list's order.
pub(crate) fn packs_left_out(game: &MountedGame) -> Vec<&str> {
    (game.listed.iter())
        .filter(|pack| pack.state == ListedPackState::LeftOut)
        .map(|pack| pack.name.as_str())
        .collect()
}

/// What `dlc-list` prints below its table: the listed packs left out, and what the mode made of each pack's change sets.
pub(crate) fn dlc_list_notes(game: &MountedGame, mode: Option<MapMode>) -> Vec<String> {
    let mut lines = Vec::new();
    let left_out = packs_left_out(game);
    if !left_out.is_empty() {
        lines.push(format!(
            "left out, {} of {} listed pack(s): {}",
            left_out.len(),
            game.listed.len(),
            left_out.join(", ")
        ));
    }
    let Some(mode) = mode else {
        return lines;
    };
    let files = || game.packs.iter().flat_map(|p| &p.data_files);
    lines.push(format!(
        "as {}: {} of {} data file(s) enabled",
        mode.label(),
        files().filter(|f| !f.disabled).count(),
        files().count()
    ));
    for pack in &game.packs {
        if !pack.change_sets_left_out.is_empty() {
            lines.push(format!(
                "  {}: left out, stated for the other mode: {}",
                pack.name_hash,
                pack.change_sets_left_out.join(", ")
            ));
        }
        if !pack.change_sets_unstated.is_empty() {
            lines.push(format!(
                "  {}: no mode stated, applied as without --mode: {}",
                pack.name_hash,
                pack.change_sets_unstated.join(", ")
            ));
        }
    }
    lines
}

/// What `dlc-list --json` prints: the mounted packs, the notes, the listed packs left out and, with a mode, what it made of each pack's change sets.
pub(crate) fn dlc_list_json(
    game: &MountedGame,
    warnings: &[String],
    mode: Option<MapMode>,
) -> serde_json::Value {
    let infos = pack_infos(game);
    let mut doc = serde_json::json!({
        "packs": infos,
        "warnings": warnings,
        "count": infos.len(),
        "leftOut": packs_left_out(game),
    });
    if let Some(mode) = mode {
        let change_sets: Vec<_> = (game.packs.iter())
            .map(|p| {
                serde_json::json!({
                    "pack": p.name_hash,
                    "leftOut": p.change_sets_left_out,
                    "unstated": p.change_sets_unstated,
                })
            })
            .collect();
        doc["mode"] = mode.label().into();
        doc["changeSets"] = change_sets.into();
    }
    doc
}

/// What `dlc-list` prints without `--json`: a row for each mounted pack, their count, then the notes.
pub(crate) fn dlc_list_text(game: &MountedGame, mode: Option<MapMode>) -> Vec<String> {
    let infos = pack_infos(game);
    let mut lines: Vec<String> = (infos.iter().enumerate())
        .map(|(idx, p)| {
            format!(
                "{:>3}  {:<26} {:<12} order={:<5} minor={:<3} files={:<5} {}",
                idx, p.name_hash, p.content_type, p.order, p.minor_order, p.data_files, p.source
            )
        })
        .collect();
    lines.push(String::new());
    lines.push(format!("{} pack(s)", infos.len()));
    lines.extend(dlc_list_notes(game, mode));
    lines
}

pub fn run_dlc_list(args: &[String]) -> ExitCode {
    let mut game: Option<PathBuf> = None;
    let mut keys: Option<PathBuf> = None;
    let mut dlc: Option<String> = None;
    let mut level: Option<String> = None;
    let mut mode: Option<String> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print_dlc_list_usage();
                return ExitCode::SUCCESS;
            }
            "--json" => json = true,
            "--game" => match next_value(args, &mut i, "--game") {
                Ok(v) => game = Some(PathBuf::from(v)),
                Err(e) => return usage_error(&e),
            },
            "--keys" => match next_value(args, &mut i, "--keys") {
                Ok(v) => keys = Some(PathBuf::from(v)),
                Err(e) => return usage_error(&e),
            },
            "--dlc" => match next_value(args, &mut i, "--dlc") {
                Ok(v) => dlc = Some(v),
                Err(e) => return usage_error(&e),
            },
            "--level" => match next_value(args, &mut i, "--level") {
                Ok(v) => level = Some(v),
                Err(e) => return usage_error(&e),
            },
            "--mode" => match next_value(args, &mut i, "--mode") {
                Ok(v) => mode = Some(v),
                Err(e) => return usage_error(&e),
            },
            other => {
                if let Some(v) = other.strip_prefix("--game=") {
                    game = Some(PathBuf::from(v));
                } else if let Some(v) = other.strip_prefix("--keys=") {
                    keys = Some(PathBuf::from(v));
                } else if let Some(v) = other.strip_prefix("--dlc=") {
                    dlc = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--level=") {
                    level = Some(v.to_string());
                } else if let Some(v) = other.strip_prefix("--mode=") {
                    mode = Some(v.to_string());
                } else {
                    return usage_error(&format!("unexpected argument: {other}"));
                }
            }
        }
        i += 1;
    }
    let (cutoff, options) = match dlc_list_mount(dlc.as_deref(), level.as_deref(), mode.as_deref())
    {
        Ok(asked) => asked,
        Err(e) => return usage_error(&e),
    };

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
    let resolved = resolve_crypto(&game_root, keys.as_deref());
    let mut warnings = resolved.notes;
    let install = match GtaInstall::from_path(&game_root) {
        Some(g) => g,
        None => return usage_error(&format!("invalid install root: {}", game_root.display())),
    };
    let game = match build_mounted_game_with(&install, resolved.provider, cutoff, options) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Error: mount failed: {e}");
            return ExitCode::from(1);
        }
    };
    warnings.extend(game.diagnostics.messages().iter().map(|d| d.to_string()));

    if json {
        match serde_json::to_string_pretty(&dlc_list_json(&game, &warnings, options.mode)) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("Error: serialize: {e}"),
        }
    } else {
        for w in &warnings {
            eprintln!("note: {w}");
        }
        for line in dlc_list_text(&game, options.mode) {
            println!("{line}");
        }
    }
    ExitCode::SUCCESS
}

fn print_dlc_list_usage() {
    eprintln!("List the DLC packs mounted for a GTA V install, in engine mount order.");
    eprintln!();
    eprintln!("Usage: openrage-cli dlc-list [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --game <dir>   GTA install root. Default: the app's selected game.");
    eprintln!("  --keys <file>  Override the keys cache (normally auto-resolved).");
    eprintln!("  --dlc=<X>      Cutoff: build number, pack nameHash, or basegame/0.");
    eprintln!("                 A name no pack states mounts every pack, with a note.");
    eprintln!("  --level <pack> Stop at this pack of the DLC list, by the name the list gives");
    eprintln!("                 it: the packs listed after it and the title update's patches");
    eprintln!("                 of them are left out and named below the table. A name the");
    eprintln!("                 list lacks is refused. Not with --dlc.");
    eprintln!("  --mode <story|multiplayer>");
    eprintln!("                 The map as story or as multiplayer: a change set a pack's");
    eprintln!("                 setup names under GROUP_MAP_SP only applies as story, one");
    eprintln!("                 under GROUP_MAP only as multiplayer. The change sets left");
    eprintln!("                 out, and the ones no group states a mode for, are named.");
    eprintln!("                 Without it every change set applies.");
    eprintln!("  --json         Machine-readable output.");
}

pub(crate) fn search_raw(
    game_root: &Path,
    provider: Arc<dyn RpfCryptoProvider>,
    matcher: &Matcher,
    cap: Cap,
    warnings: &mut Vec<String>,
) -> Result<Vec<Hit>, String> {
    let mut hits = Vec::new();
    let _ = crawl::crawl(game_root, provider, warnings, &mut |found| {
        if cap.reached(hits.len() as u64) {
            return crawl::ControlFlow::Break(());
        }
        if matcher.matches(found.leaf) {
            hits.push(Hit {
                path: found.path.to_string(),
                container: found.container.to_string(),
                file_type: None,
                disabled: None,
                pack: None,
            });
        }
        crawl::ControlFlow::Continue(())
    });
    Ok(hits)
}

pub(crate) enum Matcher {
    Any,
    Substr(String),
    Glob(or_title::vfs::Pattern),
}

impl Matcher {
    pub(crate) fn new(pattern: Option<String>) -> Self {
        match pattern {
            None => Matcher::Any,
            // ⚠ The glob is `or_title::vfs`'s rather than one of this file's
            // own, so a leaf search here and a path search elsewhere keep one
            // separator rule.
            Some(p) if p.contains('*') || p.contains('?') => {
                Matcher::Glob(or_title::vfs::Pattern::new(&p))
            }
            Some(p) => Matcher::Substr(p.to_ascii_lowercase()),
        }
    }

    pub(crate) fn matches(&self, leaf: &str) -> bool {
        match self {
            Matcher::Any => true,
            Matcher::Substr(s) => leaf.to_ascii_lowercase().contains(s.as_str()),
            Matcher::Glob(pattern) => pattern.matches_name(leaf),
        }
    }
}

fn leaf_of(path: &str) -> &str {
    let after_slash = path.rsplit(['/', '\\']).next().unwrap_or(path);
    after_slash.rsplit(':').next().unwrap_or(after_slash)
}

fn print_text(hits: &[Hit], warnings: &[String]) {
    for w in warnings {
        eprintln!("note: {w}");
    }
    for h in hits {
        let mut tags = String::new();
        if let Some(ft) = &h.file_type {
            tags.push_str(&format!("  [{ft}"));
            if h.disabled == Some(true) {
                tags.push_str(", disabled");
            }
            tags.push(']');
        }
        let pack = match &h.pack {
            Some(p) => format!("  pack:{p}"),
            None => String::new(),
        };
        println!("{}{}{}  <= {}", h.path, tags, pack, h.container);
    }
    println!("\n{} match(es)", hits.len());
}

fn print_count_json(count: usize, warnings: &[String]) {
    let doc = serde_json::json!({ "count": count, "warnings": warnings });
    match serde_json::to_string_pretty(&doc) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("Error: serialize results: {e}"),
    }
}

fn print_json(hits: &[Hit], warnings: &[String]) {
    let doc = serde_json::json!({
        "hits": hits,
        "warnings": warnings,
        "count": hits.len(),
    });
    match serde_json::to_string_pretty(&doc) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("Error: serialize results: {e}"),
    }
}

fn print_usage() {
    eprintln!("Search file names across a GTA V install (RPFs, nested RPFs, folders)");
    eprintln!();
    eprintln!("Usage: openrage-cli search [pattern] [options]");
    eprintln!();
    eprintln!("Arguments:");
    eprintln!("  pattern                Name to match (case-insensitive substring, or a glob");
    eprintln!("                         with * / ?). Optional when --fileTypes is given.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --game <dir>           GTA install root. Default: the app's selected game");
    eprintln!("                         (from prefs.json).");
    eprintln!("  --keys <file>          Override the keys cache. Normally auto-resolved from");
    eprintln!("                         the shared cache / gta5.exe.");
    eprintln!("  --mounted              Simulate the game's mount order instead of a raw crawl.");
    eprintln!("  --dlc=<X>              (mounted) Cutoff: build number (2699), pack nameHash");
    eprintln!("                         (mpsum2), or basegame/0. Default: all DLC.");
    eprintln!("  --fileTypes=<A,B,...>  (mounted) Filter to content.xml dataFiles of these");
    eprintln!("                         fileTypes, e.g. WEAPONINFO_FILE.");
    eprintln!("  --ext=<a,b,...>        Keep only these file extensions (e.g. meta,ypt).");
    eprintln!("  --device=<d>           (mounted) Keep hits whose device prefix contains <d>");
    eprintln!("                         (e.g. common, update, dlc_mpheist3).");
    eprintln!("  --count-only           Print only the number of matches.");
    eprintln!("  --json                 Machine-readable output.");
    eprintln!();
    eprintln!("Content search (looks inside files; raw crawl only):");
    eprintln!("  --text <s>             Find a string. Use --ext to say what to open.");
    eprintln!("  --hex <bytes>          Find bytes: `52504637`, `52 50 46 37`, `0x52504637`.");
    eprintln!("  --utf16                Match the UTF-16 spelling of --text.");
    eprintln!("  --case-sensitive       Default is ASCII case-insensitive.");
    eprintln!("  --under <prefix>       Only search under this container or path prefix.");
    eprintln!(
        "  --max-hits <n>         Stop after this many matching files. Default {DEFAULT_MAX_HITS}."
    );
    eprintln!("                         `--max-hits={UNLIMITED}` lifts it. A run that hits the");
    eprintln!("                         ceiling says so, and its count is then not a total.");
    eprintln!("  --max-file-size <n>    Skip files bigger than this. Default 64M.");
    eprintln!("  --no-preview           Do not print a hex window around the first hit.");
    eprintln!();
    eprintln!("Hits print as they are found and Ctrl-C stops the walk and still reports.");
    eprintln!();
    eprintln!("  --verify               Parse every readable file and report what refused.");
    eprintln!("                         A measurement of our readers, not of the game.");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  openrage-cli search railgun");
    eprintln!("  openrage-cli search --mounted --dlc=2699 --fileTypes=WEAPONINFO_FILE");
    eprintln!("  openrage-cli search \"*.ypt\" --mounted --dlc=basegame");
    eprintln!("  openrage-cli search --text WEAPON_RAILGUN --ext=meta");
    eprintln!("  openrage-cli search --hex 52504637 --under x64a.rpf --max-hits 20");
    eprintln!();
    eprintln!("Note: --fileTypes covers base-game registries (startup/default/levels/gta5.meta),");
    eprintln!(
        "the title update, and DLC content.xml files, so --dlc=basegame still lists base files."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse_args(&owned).map(|p| p.expect("not a help request"))
    }

    /// ⚠ **The default is a ceiling.** A `0`-means-*unlimited* default is the
    /// shape that let one process hold 38.4 GB of a 61.6 GB machine.
    #[test]
    fn a_search_has_a_hit_ceiling_without_being_asked() {
        let args = parse(&["anything"]).expect("parses");
        assert_eq!(args.max_hits.limit(), Some(DEFAULT_MAX_HITS));
        assert!(!args.max_hits.is_unlimited());
        assert!(args.max_hits.reached(DEFAULT_MAX_HITS));
    }

    #[test]
    fn the_ceiling_lifts_only_when_it_is_spelled_out() {
        assert!(parse(&["x", "--max-hits=unlimited"])
            .expect("parses")
            .max_hits
            .is_unlimited());
        assert_eq!(
            parse(&["x", "--max-hits", "7"])
                .expect("parses")
                .max_hits
                .limit(),
            Some(7)
        );
        let why = match parse(&["x", "--max-hits=0"]) {
            Err(why) => why,
            Ok(_) => panic!("zero is not a ceiling and must not parse as one"),
        };
        assert!(why.contains("unlimited"), "{why}");
    }

    /// The six cases `or_title::vfs::glob` asserts against its own entry point,
    /// asked of this caller through `Matcher`.
    #[test]
    fn glob_matches_star_and_question() {
        let m = |p: &str| Matcher::new(Some(p.to_string()));
        assert!(m("*.ypt").matches("scr_foo.ypt"));
        assert!(m("weapon_*.meta").matches("weapon_railgunxm3.meta"));
        assert!(m("a?c").matches("abc"));
        assert!(!m("a?c").matches("ac"));
        assert!(!m("*.ypt").matches("foo.ytd"));
        assert!(m("*").matches("anything"));
    }

    /// ⚠ A leaf search stays a leaf search even when the pattern has a `/`:
    /// `search` matches file names, and the `--fileTypes` filter beside it
    /// assumes it.
    #[test]
    fn a_leaf_search_does_not_become_a_path_search() {
        let m = Matcher::new(Some("data/*.ypt".to_string()));
        assert!(
            m.matches("scr_foo.ypt"),
            "the last segment is what a leaf search compares"
        );
    }

    #[test]
    fn matcher_substr_is_case_insensitive() {
        let m = Matcher::new(Some("Railgun".into()));
        assert!(m.matches("weapon_RAILGUNxm3.meta"));
        assert!(!m.matches("pistol.meta"));
    }

    #[test]
    fn leaf_strips_dirs_and_device() {
        assert_eq!(leaf_of("dlc_x:/common/data/foo.meta"), "foo.meta");
        assert_eq!(leaf_of("dlc_x:bar.meta"), "bar.meta");
        assert_eq!(leaf_of("plain.txt"), "plain.txt");
    }

    fn hit(path: &str) -> Hit {
        Hit {
            path: path.into(),
            container: String::new(),
            file_type: None,
            disabled: None,
            pack: None,
        }
    }

    #[test]
    fn ext_filter_keeps_matching_extensions() {
        let mut hits = vec![
            hit("common:/data/ai/weapons.meta"),
            hit("platform:/textures/foo.ytd"),
            hit("common:/data/noext"),
        ];
        let filter = HitFilter {
            ext: split_exts(".META,ypt"),
            device: None,
        };
        apply_filter(&mut hits, &filter);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "common:/data/ai/weapons.meta");
    }

    #[test]
    fn device_filter_matches_prefix_substring() {
        let mut hits = vec![
            hit("common:/data/ai/weapons.meta"),
            hit("dlc_mpHeist3CRC:/common/data/ai/weapon_navyrevolver.meta"),
            hit("update:/common/data/gta5.meta"),
        ];
        let filter = HitFilter {
            ext: Vec::new(),
            device: Some("mpheist3".into()),
        };
        apply_filter(&mut hits, &filter);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.starts_with("dlc_mpHeist3CRC:"));
    }

    #[test]
    fn empty_filter_is_noop() {
        let mut hits = vec![hit("a:/x.meta"), hit("b:/y.ytd")];
        apply_filter(&mut hits, &HitFilter::default());
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn split_exts_normalizes() {
        assert_eq!(split_exts(".Meta, YPT ,,ytd"), vec!["meta", "ypt", "ytd"]);
        assert!(split_exts("").is_empty());
    }

    #[test]
    fn search_raw_finds_loose_files_recursively() {
        use or_io::container::rpf::crypto::NoOpCryptoProvider;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("sub").join("inner")).unwrap();
        std::fs::write(root.join("weapon_railgun.meta"), b"x").unwrap();
        std::fs::write(
            root.join("sub").join("inner").join("weapon_pistol.meta"),
            b"x",
        )
        .unwrap();
        std::fs::write(root.join("sub").join("readme.txt"), b"x").unwrap();

        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let matcher = Matcher::new(Some("weapon_".into()));
        let mut warnings = Vec::new();
        let mut hits = search_raw(
            root,
            provider,
            &matcher,
            Cap::new(DEFAULT_MAX_HITS, CapUnit::Count("hit(s)")),
            &mut warnings,
        )
        .expect("search");

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(hits.len(), 2);
        hits.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(hits[0].path, "sub/inner/weapon_pistol.meta");
        assert_eq!(hits[0].container, "<loose>");
        assert_eq!(hits[1].path, "weapon_railgun.meta");

        let matcher = Matcher::new(Some("*.txt".into()));
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let mut warnings = Vec::new();
        let hits = search_raw(
            root,
            provider,
            &matcher,
            Cap::new(DEFAULT_MAX_HITS, CapUnit::Count("hit(s)")),
            &mut warnings,
        )
        .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "sub/readme.txt");
    }

    #[test]
    fn cutoff_from_build_number() {
        assert_eq!(resolve_cutoff(Some("0")), DlcCutoff::BaseGame);
        assert_eq!(resolve_cutoff(Some("basegame")), DlcCutoff::BaseGame);
        assert_eq!(resolve_cutoff(None), DlcCutoff::All);
        assert_eq!(
            resolve_cutoff(Some("2699")),
            DlcCutoff::NameHash("mpsum2".into())
        );
        assert_eq!(
            resolve_cutoff(Some("2750")),
            DlcCutoff::NameHash("mpsum2".into())
        );
        assert_eq!(
            resolve_cutoff(Some("2802")),
            DlcCutoff::NameHash("mpchristmas3".into())
        );
        assert_eq!(
            resolve_cutoff(Some("3407")),
            DlcCutoff::NameHash("mp2024_02".into())
        );
        assert_eq!(
            resolve_cutoff(Some("3600")),
            DlcCutoff::NameHash("mp2025_01".into())
        );
        assert_eq!(
            resolve_cutoff(Some("3800")),
            DlcCutoff::NameHash("mp2025_02".into())
        );
        assert_eq!(resolve_cutoff(Some("1000")), DlcCutoff::BaseGame);
        assert_eq!(
            resolve_cutoff(Some("mptuner")),
            DlcCutoff::NameHash("mptuner".into())
        );
    }
}

#[cfg(test)]
#[path = "dlc_list_tests.rs"]
mod dlc_list_tests;
