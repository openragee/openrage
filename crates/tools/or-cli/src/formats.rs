//! `openrage-cli formats`: what an install contains, per extension, and
//! whether OpenRage can open any of it.
//!
//! ⚠ **The population is the install, not the corpus.** Every scoping
//! question this answers — is this format worth a reader, is it metadata or a
//! paged asset, does a reader already exist — has a different answer against
//! 246 staged files than against 396,452 shipped ones.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use or_io::awc::AwcKey;
use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_io::gta5::formats::Format;
use or_io::meta::binary::Container;
use or_io::meta::SchemaLibrary;
use or_title::install::crawl::{self, ControlFlow};

use super::census_cap::{Cap, CapUnit};
use super::keys::resolve_crypto;
use super::search::resolve_game_root;
use super::usage_error;

/// How many files of each extension are opened. The count per extension is
/// exact; the classification is a sample, and the report says which is which.
const DEFAULT_SAMPLE: usize = 48;

/// How many bytes of file content the classification may read before it stops
/// sampling.
///
/// ⚠ `--all` with no ceiling parses the whole install (on the order of
/// 396,000 files); run detached against a retail install it reached **38.4 GB
/// of a 61.6 GB machine** and took the machine down.
///
/// ⚠ **It bounds bytes rather than files**: an install's `.ytd` and `.ydr`
/// differ by four orders of magnitude in size and it is the large ones that
/// cost, so a file-count cap would be generous on textures and useless on
/// drawables.
///
/// Only the classification stops; the per-extension file counts stay exact.
const SAMPLE_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const BUDGET_FLAG: &str = "--budget-gib";

fn byte_budget() -> Cap {
    Cap::new(SAMPLE_BUDGET_BYTES, CapUnit::Gibibytes)
}

/// What a survey read, and whether it stopped short of what it was asked for.
#[derive(Default)]
struct Sampling {
    bytes_read: u64,
    files_sampled: usize,
    /// The budget, once it has been spent. `None` means the run classified
    /// everything it was asked to.
    stopped_at: Option<u64>,
}

/// Caps bytes classified where `names` and `search` cap a count of results;
/// [`Cap`](super::census_cap::Cap) gives all three the same rules: zero is
/// refused, lifting the ceiling is spelled as a word, and a run that stopped
/// short says so.
impl Sampling {
    /// Whether the ceiling has already been reached, and so no further file
    /// should be classified.
    fn exhausted(&self) -> bool {
        self.stopped_at.is_some()
    }

    /// Charge one classified file against the ceiling.
    ///
    /// ⚠ **The file that crosses the budget is charged and kept.** Discarding
    /// it would make the reported figure depend on walk order in a way
    /// nothing could reproduce.
    fn spend(&mut self, bytes: usize, budget: Cap) {
        self.files_sampled += 1;
        self.bytes_read += bytes as u64;
        if budget.reached(self.bytes_read) {
            self.stopped_at = budget.limit();
        }
    }
}

struct Survey {
    facts: BTreeMap<String, ExtensionFacts>,
    sampling: Sampling,
}

#[derive(Default)]
struct ExtensionFacts {
    files: usize,
    sampled: usize,
    unreadable: usize,
    /// Container label → count.
    containers: BTreeMap<String, usize>,
    /// Root type, as the metadata reader names it → count.
    roots: BTreeMap<String, usize>,
    /// Reader → sampled files it parsed. A reader that refused every file it
    /// was tried on stays listed at 0.
    readers: BTreeMap<String, usize>,
    /// Sampled files every reader tried on refused.
    refused: usize,
    /// Its extension names a resource even though nothing reads it.
    detected_only: bool,
}

impl ExtensionFacts {
    fn one(map: &mut BTreeMap<String, usize>, key: String) {
        *map.entry(key).or_default() += 1;
    }

    /// Who reads this, measured by reading it.
    ///
    /// ⚠ **Detection is not reading.** `.ypm` names a `ResourceType`
    /// and nothing opens it, so a column filled from the extension
    /// would report a reader that does not exist. This one reports what
    /// parsed.
    fn reader(&self) -> String {
        if self.sampled == 0 {
            return "not sampled".to_string();
        }
        if self.readers.is_empty() {
            let verdict = if self.detected_only {
                "detected only"
            } else {
                "none"
            };
            return verdict.to_string();
        }
        self.readers
            .iter()
            .map(|(reader, parsed)| format!("{reader} {parsed}/{}", self.sampled))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Records one sampled file: the reader that parsed it, or the one that refused it.
    fn read_by(&mut self, reader: String, parsed: bool) {
        let count = self.readers.entry(reader).or_default();
        if parsed {
            *count += 1;
        } else {
            self.refused += 1;
        }
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let mut game: Option<PathBuf> = None;
    let mut keys: Option<PathBuf> = None;
    let mut only: Vec<String> = Vec::new();
    let mut sample = DEFAULT_SAMPLE;
    let mut json = false;
    let mut budget = byte_budget();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (arg, None),
        };
        match key {
            "-h" | "--help" | "help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "--json" => json = true,
            "--all" => sample = usize::MAX,
            "--no-byte-budget" => budget = budget.lifted(),
            "--game" | "--keys" | "--ext" | "--sample" | "--budget-gib" => {
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
                    "--ext" => only = super::search::split_exts(&v),
                    "--budget-gib" => match budget.parse(BUDGET_FLAG, &v) {
                        Ok(c) => budget = c,
                        Err(why) => return usage_error(&why),
                    },
                    _ => match v.parse::<usize>() {
                        Ok(n) => sample = n,
                        Err(_) => return usage_error("--sample needs a number"),
                    },
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

    let resolved = resolve_crypto(&game_root, keys.as_deref());
    let mut warnings = resolved.notes;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = Arc::clone(&cancel);
        let _ = super::jobs::install_stop_handler(move || cancel.store(true, Ordering::Relaxed));
    }

    eprintln!(
        "openrage-cli: surveying {} (Ctrl-C stops and still reports) ...",
        game_root.display()
    );
    if budget.is_unlimited() {
        eprintln!(
            "openrage-cli: that removes the {} classification ceiling. An unbounded \
             sweep of a whole install has taken a machine down; watch it, and do not \
             leave it running unattended.",
            byte_budget().rendered_limit()
        );
    }

    let survey = survey(
        &game_root,
        resolved.provider,
        resolved.awc_key,
        &only,
        sample,
        budget,
        &cancel,
        &mut warnings,
    );
    let facts = survey.facts;

    if json {
        print_json(&facts, &survey.sampling, &warnings);
    } else {
        for w in &warnings {
            eprintln!("note: {w}");
        }
        print!("{}", report(&facts));
        print!("{}", sampling_note(&survey.sampling));
        if !crate::meta::has_schema() {
            println!(
                "\n⚠ No schema snapshot is staged, so every binary metadata root type reads \
                 as a hash and XML metadata is refused. See crates/io/or-io/src/meta/README.md."
            );
        }
    }
    ExitCode::SUCCESS
}

#[allow(clippy::too_many_arguments)]
fn survey(
    game_root: &Path,
    provider: Arc<dyn RpfCryptoProvider>,
    awc_key: AwcKey,
    only: &[String],
    sample: usize,
    budget: Cap,
    cancel: &AtomicBool,
    warnings: &mut Vec<String>,
) -> Survey {
    let mut facts: BTreeMap<String, ExtensionFacts> = BTreeMap::new();
    let mut sampling = Sampling::default();
    let library = crate::meta::schema();
    let _ = crawl::crawl(game_root, provider, warnings, &mut |found| {
        if cancel.load(Ordering::Relaxed) {
            return ControlFlow::Break(());
        }
        let Some(ext) = found
            .leaf
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
        else {
            return ControlFlow::Continue(());
        };
        if !only.is_empty() && !only.contains(&ext) {
            return ControlFlow::Continue(());
        }
        let entry = facts.entry(ext.clone()).or_default();
        entry.files += 1;
        if entry.sampled >= sample {
            return ControlFlow::Continue(());
        }
        // ⚠ The walk continues past the budget rather than breaking: breaking
        // would silently shorten the file counts too, which are the exact half
        // and cost nothing.
        if sampling.exhausted() {
            return ControlFlow::Continue(());
        }
        let Ok(bytes) = found.read() else {
            entry.unreadable += 1;
            return ControlFlow::Continue(());
        };
        entry.sampled += 1;
        sampling.spend(bytes.len(), budget);
        classify_into(entry, found.leaf, &ext, &bytes, awc_key, library);
        ControlFlow::Continue(())
    });
    Survey { facts, sampling }
}

/// What the run classified, and — when it stopped short — that it did.
///
/// ⚠ **A census that truncates silently is worse than one that refuses**,
/// because the figure it prints is quotable and wrong. This is printed on the
/// same stream as the table for that reason.
fn sampling_note(sampling: &Sampling) -> String {
    let Some(limit) = sampling.stopped_at else {
        return String::new();
    };
    format!(
        "\n⚠ TRUNCATED: classification stopped after {} file(s) and {:.1} GiB, at the \
         {:.0} GiB budget. File counts above are exact; every `reads` verdict is from \
         what was classified before the stop, and an extension reached after it reports \
         `not sampled`. Narrow with --ext, or raise it with --budget-gib, or remove it \
         with --no-byte-budget.\n",
        sampling.files_sampled,
        sampling.bytes_read as f64 / (1024.0 * 1024.0 * 1024.0),
        limit as f64 / (1024.0 * 1024.0 * 1024.0),
    )
}

fn classify_into(
    entry: &mut ExtensionFacts,
    leaf: &str,
    ext: &str,
    bytes: &[u8],
    awc_key: AwcKey,
    library: &SchemaLibrary,
) {
    let container = or_io::meta::binary::classify(bytes);
    ExtensionFacts::one(&mut entry.containers, container.label());

    let read = match read(leaf, ext, bytes, awc_key) {
        Some((reader, outcome)) => Some((reader, outcome.is_ok())),
        None => metadata(&mut entry.roots, ext, container, bytes, library),
    };
    match read {
        Some((reader, parsed)) => entry.read_by(reader, parsed),
        // Nothing read it. Say whether anything even claims to recognise it,
        // because "registered but unopenable" is its own state. ⚠ The app
        // previews a `.bik` through `or-media`, which this census does not
        // use, so it is detected rather than read.
        None => {
            entry.detected_only = ext == "bik"
                || or_io::container::resource::ResourceType::from_extension(ext).is_some();
        }
    }
}

/// The reader that takes a file named `leaf`, and whether it parsed `bytes`
/// or why it refused them; `None` when no reader takes it.
///
/// ⚠ Whether a reader takes the file hangs on its name alone, never on
/// `bytes`, so `verify::is_known` asks with none.
pub(crate) fn read(
    leaf: &str,
    ext: &str,
    bytes: &[u8],
    awc_key: AwcKey,
) -> Option<(String, Result<(), String>)> {
    use or_io::decl::File as _;
    use or_io::gta5::shaders::{fxc::GrcEffect, sga::SgaFile, sps::SpsPreset};

    if let Some(format) = or_io::gta5::formats::format(ext) {
        return Some(owned_root(format, bytes));
    }
    let (reader, outcome) = match ext {
        // ⚠ Enhanced ships its shader container as `sga_win32_60_final.awc`.
        "awc" => match or_io::awc::AwcFile::parse(bytes, awc_key) {
            Ok(_) => ("AwcFile", Ok(())),
            Err(_) if SgaFile::parse(bytes).is_ok() => ("SgaFile", Ok(())),
            Err(e) => ("AwcFile", Err(e.to_string())),
        },
        // The DLC audio containers carry their `rel_type` as the extension.
        "rel" | "dat4" | "dat10" | "dat15" | "dat16" | "dat22" | "dat54" | "dat149" | "dat150"
        | "dat151" => ("RelFile", parsed(or_io::rel::RelFile::open(bytes))),
        "gfx" => ("GfxMovie", parsed(or_io::gfx::GfxMovie::open(bytes))),
        // Binary or XML; either reads with no schema.
        "srl" => (
            "StreamingRequestList",
            parsed(or_io::meta::MetaDocument::parse(
                or_io::meta::default_library(),
                bytes.to_vec(),
            )),
        ),
        "mrf" => (
            "MvNetworkDef",
            parsed(or_io::mrf::MvNetworkDef::open(bytes)),
        ),
        "dds" => ("Dds", parsed(or_io::dds::Dds::from_bytes(bytes))),
        // A binary world `.dat` has no tag to tell it from a text one: the game names each.
        "dat" => world_dat(leaf, bytes)?,
        // The level's water is known by its file name, ahead of the metadata an `.xml` otherwise is.
        "xml" if or_io::gta5::data::water::is_water_file(leaf) => (
            "CWaterData",
            parsed(or_io::gta5::data::water::CWaterData::open(bytes)),
        ),
        "nametable" => (
            "NameTable",
            parsed(or_io::gta5::data::nametable::parse(bytes)),
        ),
        // A preset parses from any text, so one that binds no effect is refused.
        "sps" => (
            "SpsPreset",
            match std::str::from_utf8(bytes) {
                Ok(text) if !SpsPreset::parse("", text).effect.is_empty() => Ok(()),
                Ok(_) => Err("the preset binds no effect".to_string()),
                Err(e) => Err(e.to_string()),
            },
        ),
        "gxt2" => ("Gxt2", parsed(or_io::gta5::text::Gxt2::from_bytes(bytes))),
        "ypdb" => (
            "PoseMatcherDatabase",
            parsed(or_io::gta5::pose_db::PoseMatcherDatabase::from_bytes(bytes)),
        ),
        // The effect, not the DXBC scan: the scan finds blobs in anything and
        // would report a file it never understood as read.
        "fxc" => ("GrcEffect", parsed(GrcEffect::parse(bytes))),
        "ysc" => (
            "ScrProgram",
            parsed(or_io::gta5::script::ScrProgram::from_bytes(bytes)),
        ),
        _ => return None,
    };
    Some((reader.to_string(), outcome))
}

/// A reader's outcome with its value dropped and its error rendered.
fn parsed<T, E: std::fmt::Display>(outcome: Result<T, E>) -> Result<(), String> {
    outcome.map(drop).map_err(|e| e.to_string())
}

/// The binary world `.dat` reader `leaf` names, and whether it parsed `bytes`.
fn world_dat(leaf: &str, bytes: &[u8]) -> Option<(&'static str, Result<(), String>)> {
    let name = leaf.to_ascii_lowercase();
    let read = if name.starts_with("heightmap") {
        (
            "HeightMap",
            parsed(or_io::height_map::HeightMap::open(bytes)),
        )
    } else if name.starts_with("distantlights") {
        (
            "DistantLights",
            parsed(or_io::distant_lights::DistantLights::open(bytes)),
        )
    } else if name.starts_with("waterheight") {
        ("WaterMap", parsed(or_io::water_map::WaterMap::open(bytes)))
    } else if name.starts_with("worldblanket") {
        (
            "WorldBlanket",
            parsed(or_io::world_blanket::WorldBlanket::open(bytes)),
        )
    } else if name.starts_with("navprecalc") {
        (
            "NavPrecalc",
            parsed(or_io::nav_precalc::NavPrecalc::open(bytes)),
        )
    } else if name.ends_with("_cache_y.dat") {
        (
            "CacheFile",
            parsed(or_io::cache_loader::CacheFile::open(bytes)),
        )
    } else if name == "metadata.dat" {
        (
            "SocialClubMetadata",
            parsed(or_io::social_club_metadata::SocialClubMetadata::open(bytes)),
        )
    } else if name == "audioworldsectorsinfo.dat" {
        (
            "AudioWorldSectors",
            parsed(or_io::audio_world_sectors::AudioWorldSectors::open(bytes)),
        )
    } else {
        return None;
    };
    Some(read)
}

/// A declared format's file, read as the owned root its RSC7 version names
/// and reported as that root and its edition.
fn owned_root(format: &Format, bytes: &[u8]) -> (String, Result<(), String>) {
    use or_io::container::resource::{ResourceEnvelope, Rsc7};
    use or_io::decl::{File, Gen8, Gen9};
    use or_io::gta5::gen8::GrcTexture;
    use or_io::gta5::gen9::GrcTextureG9;
    use or_io::gta5::{
        CNavMesh, CPathRegion, CVehicleRecording, CharacterCloth, CrClipDictionary,
        CrExpressionsDictionary, CrFrameFilterDictionary, FwWaypointRecordingRoute, GtaDrawable,
        GtaFragType, PgDictionary, PhBoundAny, PtxFxList,
    };

    fn load<T: File>(bytes: &[u8]) -> Result<(), String> {
        parsed(T::from_bytes(bytes))
    }

    let named = Rsc7::peek(bytes)
        .map_err(|e| e.to_string())
        .and_then(|seal| {
            format
                .target_of_version(seal.version())
                .map_err(|e| e.to_string())
        });
    // A version no target declares is refused by the format's first reader.
    let target = named
        .as_ref()
        .ok()
        .copied()
        .or_else(|| format.targets().next());
    let Some(target) = target else {
        return (
            format.ext.to_string(),
            Err("no target declared".to_string()),
        );
    };
    let outcome = named.and_then(|_| match (format.ext, target.is_gen9()) {
        ("yfd", _) => load::<CrFrameFilterDictionary>(bytes),
        ("yed", _) => load::<CrExpressionsDictionary>(bytes),
        ("ynd", _) => load::<CPathRegion>(bytes),
        ("ynv", _) => load::<CNavMesh>(bytes),
        ("ybn", _) => load::<PhBoundAny>(bytes),
        ("ybd", _) => load::<PgDictionary<PhBoundAny>>(bytes),
        ("yld", _) => load::<PgDictionary<CharacterCloth>>(bytes),
        ("ycd", _) => load::<CrClipDictionary>(bytes),
        ("ytd", false) => load::<PgDictionary<GrcTexture>>(bytes),
        ("ytd", true) => load::<PgDictionary<GrcTextureG9>>(bytes),
        ("ydr", false) => load::<GtaDrawable<Gen8>>(bytes),
        ("ydr", true) => load::<GtaDrawable<Gen9>>(bytes),
        ("ydd", false) => load::<PgDictionary<GtaDrawable<Gen8>>>(bytes),
        ("ydd", true) => load::<PgDictionary<GtaDrawable<Gen9>>>(bytes),
        ("yft", false) => load::<GtaFragType<Gen8>>(bytes),
        ("yft", true) => load::<GtaFragType<Gen9>>(bytes),
        ("ypt", false) => load::<PtxFxList<Gen8>>(bytes),
        ("ypt", true) => load::<PtxFxList<Gen9>>(bytes),
        ("yvr", _) => load::<CVehicleRecording>(bytes),
        ("ywr", _) => load::<FwWaypointRecordingRoute>(bytes),
        (ext, _) => Err(format!("no owned reader for .{ext}")),
    });
    let root = format
        .root_name(target)
        .unwrap_or_else(|_| format.ext.to_string());
    (format!("{root} ({})", target.edition_label()), outcome)
}

/// A metadata file read through `or_io::meta`, with the root type it
/// declares counted in `roots`: named when the schema snapshot knows it and
/// `hash_<hex>` when it does not.
fn metadata(
    roots: &mut BTreeMap<String, usize>,
    ext: &str,
    container: Container,
    bytes: &[u8],
    library: &SchemaLibrary,
) -> Option<(String, bool)> {
    let (reader, parsed) = match container {
        Container::Meta(format) => {
            let mut diags = or_io::meta::diagnostics::Diagnostics::lenient();
            let parsed = or_io::meta::binary::parse_meta_binary_with(library, bytes, &mut diags)
                .map(|instance| {
                    or_io::meta::binary::root_types::name_for(
                        u32::from_str_radix(instance.type_name.trim_start_matches("0x"), 16)
                            .unwrap_or(0),
                    )
                    .map(str::to_string)
                    .unwrap_or(instance.type_name)
                });
            (format!("meta {}", format.label()), parsed)
        }
        _ if matches!(ext, "meta" | "xml") => (
            "meta XML".to_string(),
            or_io::meta::instance::parse_meta(library, bytes).map(|instance| instance.type_name),
        ),
        _ => return None,
    };
    let root = match &parsed {
        Ok(root) => root.clone(),
        Err(e) => format!("<unreadable: {}>", first_clause(&e.to_string())),
    };
    ExtensionFacts::one(roots, root);
    Some((reader, parsed.is_ok()))
}

fn first_clause(message: &str) -> String {
    message
        .split(&[':', ';'][..])
        .next()
        .unwrap_or(message)
        .trim()
        .to_string()
}

fn report(facts: &BTreeMap<String, ExtensionFacts>) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<10} {:>9} {:>8}  {:<16} {:<16} {}\n",
        "ext", "files", "sampled", "container", "reads", "root type"
    ));
    for (ext, f) in facts {
        out.push_str(&format!(
            "{:<10} {:>9} {:>8}  {:<16} {:<16} {}\n",
            ext,
            f.files,
            f.sampled,
            join_counts(&f.containers),
            f.reader(),
            join_counts(&f.roots),
        ));
        if f.unreadable > 0 {
            out.push_str(&format!("{:<8} {:>9} unreadable\n", "", f.unreadable));
        }
    }
    let files: usize = facts.values().map(|f| f.files).sum();
    let sampled: usize = facts.values().map(|f| f.sampled).sum();
    out.push_str(&format!(
        "\n{} extension(s), {files} file(s), {sampled} opened\n",
        facts.len()
    ));
    out
}

/// `a x3, b` — the value when there is one, and the distribution when the
/// sample disagrees with itself.
fn join_counts(counts: &BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "—".to_string();
    }
    let mut pairs: Vec<(&String, &usize)> = counts.iter().collect();
    pairs.sort_by_key(|(name, count)| (std::cmp::Reverse(**count), (*name).clone()));
    if pairs.len() == 1 {
        return pairs[0].0.clone();
    }
    pairs
        .iter()
        .map(|(name, count)| format!("{name} x{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_json(facts: &BTreeMap<String, ExtensionFacts>, sampling: &Sampling, warnings: &[String]) {
    let rows: Vec<serde_json::Value> = facts
        .iter()
        .map(|(ext, f)| {
            serde_json::json!({
                "ext": ext,
                "files": f.files,
                "sampled": f.sampled,
                "unreadable": f.unreadable,
                "reads": f.reader(),
                "parsed": f.readers.values().sum::<usize>(),
                "refused": f.refused,
                "readers": f.readers,
                "detectedOnly": f.detected_only,
                "containers": f.containers,
                "rootTypes": f.roots,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "extensions": rows,
        "schemaSnapshot": crate::meta::has_schema(),
        // ⚠ A consumer reading this as a census has to be able to see that
        // the sample stopped, so `truncated` is a top-level key rather than
        // something inferable from comparing `files` against `sampled`.
        "truncated": sampling.stopped_at.is_some(),
        "bytesClassified": sampling.bytes_read,
        "filesClassified": sampling.files_sampled,
        "budgetBytes": sampling.stopped_at,
        "warnings": warnings,
    });
    match serde_json::to_string_pretty(&doc) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("Error: serialize: {e}"),
    }
}

fn print_usage() {
    eprintln!("Survey an install: what is in it, and what OpenRage can open.");
    eprintln!();
    eprintln!("Usage: openrage-cli formats [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --game <dir>     GTA install root. Default: the app's selected game.");
    eprintln!(
        "  --budget-gib <n> Raise or lower the {} classification ceiling, or",
        byte_budget().rendered_limit()
    );
    eprintln!("                   `unlimited`. ⚠ `0` is an error, not `unlimited`.");
    eprintln!("  --no-byte-budget The same as `--budget-gib unlimited`. An unbounded sweep");
    eprintln!("                   of a whole install has taken a machine down; do not");
    eprintln!("                   leave one unattended.");
    eprintln!("  --keys <file>    Override the keys cache (normally auto-resolved).");
    eprintln!("  --ext=<a,b>      Only these extensions.");
    eprintln!("  --sample <n>     Files opened per extension. Default 48.");
    eprintln!("  --all            Open every file of every extension. ⚠ This is the whole");
    eprintln!("                   install — hundreds of thousands of files — and it stops at");
    eprintln!("                   the classification ceiling below and says that it did.");
    eprintln!("  --json           Machine-readable output.");
    eprintln!();
    eprintln!("The file count is exact; the container and root type are a sample, and the");
    eprintln!("report prints both so a mixed extension shows as a distribution.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(
        sampled: usize,
        reader: Option<&'static str>,
        parsed: usize,
        refused: usize,
    ) -> ExtensionFacts {
        ExtensionFacts {
            files: sampled * 100,
            sampled,
            readers: reader
                .map(|r| BTreeMap::from([(r.to_string(), parsed)]))
                .unwrap_or_default(),
            refused,
            ..Default::default()
        }
    }

    #[test]
    fn the_ceiling_stops_the_classification_and_not_the_count() {
        let mut sampling = Sampling::default();
        let budget = Cap::at(1000, CapUnit::Gibibytes);

        sampling.spend(400, budget);
        assert!(!sampling.exhausted(), "under the budget");
        sampling.spend(400, budget);
        assert!(!sampling.exhausted(), "still under");

        sampling.spend(400, budget);
        assert!(sampling.exhausted(), "1200 of 1000 is over");
        assert_eq!(sampling.stopped_at, Some(1000));
        // The file that crossed it is kept, so the reported figure is what
        // was actually read rather than what fitted.
        assert_eq!(sampling.bytes_read, 1200);
        assert_eq!(sampling.files_sampled, 3);
    }

    /// ⚠ `0` is an error here as in `names` and `search`: a flag that
    /// defaults to zero and means *no ceiling* is one forgotten initialiser
    /// from an unbounded run.
    #[test]
    fn zero_gibibytes_is_refused_the_way_zero_hits_is() {
        // ⚠ The last one is a *fraction* that rounds to zero bytes. It reads
        // as a request for some budget and is a ceiling the first file
        // crosses, so it is refused for the same reason `0` is.
        for zero in ["0", "0.0", "0.0000000001"] {
            let why = byte_budget()
                .parse(BUDGET_FLAG, zero)
                .expect_err("zero is not a ceiling");
            assert!(why.contains("unlimited"), "{zero}: {why}");
        }
        assert!(byte_budget()
            .parse(BUDGET_FLAG, "unlimited")
            .unwrap()
            .is_unlimited());
        assert_eq!(
            byte_budget().parse(BUDGET_FLAG, "0.5").unwrap().limit(),
            Some(SAMPLE_BUDGET_BYTES / 4)
        );
    }

    /// The two spellings of the override are the same override.
    #[test]
    fn no_byte_budget_and_unlimited_are_one_state() {
        assert_eq!(
            byte_budget().lifted(),
            byte_budget().parse(BUDGET_FLAG, "unlimited").unwrap()
        );
    }

    #[test]
    fn no_budget_never_stops() {
        let mut sampling = Sampling::default();
        for _ in 0..1000 {
            sampling.spend(usize::from(u16::MAX), byte_budget().lifted());
        }
        assert!(!sampling.exhausted());
        assert!(sampling_note(&sampling).is_empty());
    }

    #[test]
    fn a_truncated_run_says_so_and_names_the_way_out() {
        let mut sampling = Sampling::default();
        sampling.spend(3 * 1024 * 1024 * 1024, byte_budget());
        let note = sampling_note(&sampling);

        assert!(note.contains("TRUNCATED"), "{note}");
        assert!(note.contains("File counts above are exact"), "{note}");
        assert!(note.contains("not sampled"), "{note}");
        for way_out in ["--ext", "--budget-gib", "--no-byte-budget"] {
            assert!(note.contains(way_out), "{way_out} is not offered: {note}");
        }
    }

    #[test]
    fn an_untruncated_run_prints_nothing_at_all() {
        let mut sampling = Sampling::default();
        sampling.spend(16, byte_budget());
        assert_eq!(sampling_note(&sampling), "");
    }

    #[test]
    fn a_reader_verdict_is_what_parsed_rather_than_what_was_recognised() {
        assert_eq!(facts(24, Some("RelFile"), 24, 0).reader(), "RelFile 24/24");
        assert_eq!(
            facts(24, Some("meta RBM"), 24, 0).reader(),
            "meta RBM 24/24"
        );
        // A reader that exists and mostly fails is not the same as no reader.
        assert_eq!(facts(24, Some("RelFile"), 3, 21).reader(), "RelFile 3/24");
        assert_eq!(facts(24, None, 0, 0).reader(), "none");
    }

    /// An extension two readers share names each with what it parsed.
    #[test]
    fn each_reader_that_parsed_a_sample_is_named() {
        let mut f = facts(3, None, 0, 0);
        f.read_by("AwcFile".into(), true);
        f.read_by("AwcFile".into(), true);
        f.read_by("SgaFile".into(), true);
        assert_eq!(f.reader(), "AwcFile 2/3, SgaFile 1/3");
        assert_eq!(f.refused, 0);

        let mut refused = facts(1, None, 0, 0);
        refused.read_by("AwcFile".into(), false);
        assert_eq!(refused.reader(), "AwcFile 0/1");
        assert_eq!(refused.refused, 1);
    }

    /// ⚠ `.ypm` names a `ResourceType` and nothing opens it, so "detected"
    /// must not render as a reader.
    #[test]
    fn a_registered_format_nothing_opens_says_detected_only() {
        let mut f = facts(24, None, 0, 0);
        f.detected_only = true;
        assert_eq!(f.reader(), "detected only");
    }

    /// ⚠ An extension that was never opened must not read as "nothing opens
    /// this" — that is the difference between a measurement and a gap.
    #[test]
    fn an_unsampled_extension_says_so_rather_than_claiming_no_reader() {
        assert_eq!(facts(0, None, 0, 0).reader(), "not sampled");
    }

    #[test]
    fn a_mixed_sample_reports_the_distribution_rather_than_the_winner() {
        let mut counts = BTreeMap::new();
        ExtensionFacts::one(&mut counts, "RBM".into());
        assert_eq!(join_counts(&counts), "RBM");

        for _ in 0..3 {
            ExtensionFacts::one(&mut counts, "PSO".into());
        }
        assert_eq!(join_counts(&counts), "PSO x3, RBM x1");
        assert_eq!(join_counts(&BTreeMap::new()), "—");
    }

    #[test]
    fn a_report_names_the_exact_count_and_the_sample_separately() {
        let mut f = facts(48, Some("meta RBM"), 48, 0);
        f.files = 19387;
        ExtensionFacts::one(&mut f.containers, "RBM".into());
        ExtensionFacts::one(&mut f.roots, "CMapData".into());
        let mut all = BTreeMap::new();
        all.insert("ymap".to_string(), f);

        let text = report(&all);
        assert!(text.contains("19387"), "{text}");
        assert!(text.contains("CMapData"), "{text}");
        assert!(text.contains("meta RBM 48/48"), "{text}");
        assert!(
            text.contains("1 extension(s), 19387 file(s), 48 opened"),
            "{text}"
        );
    }

    #[test]
    fn a_reader_error_is_reduced_to_its_first_clause() {
        assert_eq!(
            first_clause("rbm header: PRD0 sentinel mismatch (got 0x5, expected 0x50524430)"),
            "rbm header"
        );
        assert_eq!(first_clause("plain"), "plain");
    }
}
