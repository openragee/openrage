//! `openrage-cli depends` — what an asset needs, and where a model is put.
//!
//! ⚠ **One parsed tree at a time, everywhere below.** A retail install's
//! `.ytyp` and `.ymap` value trees do not fit in memory together — a
//! `world check` run that held them reached 38 GB and was killed — so every
//! loop here folds into scalars and drops the tree at the end of its own
//! iteration.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use or_depends::{Census, DependencySet, Derivable, Resolver, Stated, RULES};
use or_io::decl::Gen8;
use or_io::gta5::gen8::GrcTexture;
use or_io::gta5::{File as _, GtaDrawable, GtaFragType, PgDictionary};
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{Diagnostics, ErrorMode, MetaInstance};
use or_world_gta5::{
    ArchetypeAssetIndex, Catalogue, InstallTables, Kind, ModelContents, PlacementIndex, Via,
};
use or_world_gta5::{MapData, MapTypes};

use super::world::{flag_value, limit_of, name_or_hash};

pub fn run(args: &[String]) -> ExitCode {
    let Some(sub) = args.first() else {
        print_usage();
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    match sub.as_str() {
        "of" => cmd_of(rest),
        "placements" => cmd_placements(rest),
        "census" => cmd_census(rest),
        "textures" => cmd_textures(rest),
        "export" => cmd_export(rest),
        "rules" => cmd_rules(),
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Error: unknown `depends` subcommand '{other}'");
            print_usage();
            ExitCode::from(2)
        }
    }
}

/// Flags that take a value, so the value is not mistaken for the subject.
///
/// ⚠ **Not cosmetic.** A bare "first argument that does not start with a
/// dash" reads `--game <path>` as the subject and hashes the install path,
/// which answers confidently about an archetype nothing declares.
const VALUED_FLAGS: &[&str] = &["--game", "--keys", "--limit", "--sample", "--out"];

/// The name an archetype or asset subject hashes from: an asset path's stem,
/// so `props/prop_bench_01.ydr` asks for `prop_bench_01`.
fn subject_name(subject: &str) -> &str {
    let leaf = subject.rsplit(['/', '\\']).next().unwrap_or(subject);
    match leaf.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => stem,
        _ => leaf,
    }
}

/// The arguments that are not a flag and not a flag's value.
fn subjects(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if VALUED_FLAGS.contains(&arg.as_str()) {
            skip = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        out.push(arg.as_str());
    }
    out
}

/// ⚠ **Dispatch on the bytes, never on the extension.** `gtxd.meta` is XML
/// and `gtxd.ymt` is a paged binary saying the same thing, and a run that
/// fed both to the binary reader read **0** texture-dictionary parents out
/// of an install that ships them.
///
/// ⚠ **Every shipped `gtxd.meta` opens with a UTF-8 BOM**, so a sniff for a
/// leading `<` finds `0xEF` and sends XML to the binary reader — which is
/// the same 0 with a different cause.
///
/// Read with the schema the host installed: the command's, or the app's.
fn parse(bytes: &[u8]) -> Option<MetaInstance> {
    let schema = or_io::meta::default_library();
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if body.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<') {
        return or_io::meta::instance::parser::parse_meta(schema, body).ok();
    }
    let mut diags = Diagnostics::new(ErrorMode::Lenient);
    parse_meta_binary_full(schema, bytes, &mut diags)
        .ok()
        .map(|(inst, _)| inst)
}

/// Everything a resolver needs, built from a mount in one pass per table: what
/// `depends` answers from, and the app's Uses and Placed by.
pub struct DependencyIndex {
    pub mount: or_title::MountedGame,
    pub catalogue: Catalogue,
    archetypes: ArchetypeAssetIndex,
    tables: InstallTables,
    contents: ModelContents,
    /// The `.ymap` the engine would load, one logical path per leaf.
    maps: Vec<String>,
    type_files: usize,
}

/// Mount the install at `game_root`, then read what a resolver needs from it.
pub(crate) fn load(game_root: &Path, keys: Option<&str>) -> Result<DependencyIndex, String> {
    // Installs the command's snapshot, which `parse` reads every table with.
    crate::meta::schema();
    let mount = super::keys::mount_all(game_root, keys.map(Path::new))?;
    Ok(DependencyIndex::read(mount, &mut |line| {
        eprintln!("{line}")
    }))
}

impl DependencyIndex {
    /// Index the entry table, read the three metadata families, then the
    /// model bodies an entry table cannot settle, saying what each step found
    /// through `said`.
    ///
    /// ⚠ **Keyed by leaf so the winner is chosen before anything is parsed.** A
    /// retail install ships many copies of most world files and `update.rpf`
    /// replaces base-game maps wholesale, frequently with an empty stub, so
    /// reading every copy answers about files the engine never loads.
    pub fn read(mount: or_title::MountedGame, said: &mut dyn FnMut(String)) -> Self {
        let started = std::time::Instant::now();
        let mut catalogue = Catalogue::new();
        mount.for_each_entry(|facts| catalogue.index(facts));
        said(format!(
            "catalogue: {} name(s) from {} entr(ies) in {:.2}s, {} shadowed by a later pack",
            catalogue.len(),
            catalogue.entries(),
            started.elapsed().as_secs_f64(),
            catalogue.shadowed()
        ));

        // One winner per leaf, for the two families that are read as trees.
        let mut type_winners: BTreeMap<String, String> = BTreeMap::new();
        let mut map_winners: BTreeMap<String, String> = BTreeMap::new();
        let mut manifests: Vec<String> = Vec::new();
        let mut relationships: Vec<String> = Vec::new();
        mount.for_each_file(|logical, _container, leaf| {
            let lower = leaf.to_ascii_lowercase();
            if lower.ends_with(".ytyp") {
                type_winners.insert(lower, logical.to_string());
            } else if lower.ends_with(".ymap") {
                map_winners.insert(lower, logical.to_string());
            } else if lower.ends_with(".ymf") {
                manifests.push(logical.to_string());
            } else if lower.contains("gtxd")
                && (lower.ends_with(".meta") || lower.ends_with(".ymt"))
            {
                relationships.push(logical.to_string());
            }
        });

        let mut tables = InstallTables::new();
        for path in &relationships {
            let Some(bytes) = mount.vfs.open(path) else {
                continue;
            };
            let Some(root) = parse(&bytes) else { continue };
            if or_world_gta5::tables::is_txd_relationships(&root) {
                tables.add_relationships(&root);
            }
        }
        for path in &manifests {
            let Some(bytes) = mount.vfs.open(path) else {
                continue;
            };
            let Some(root) = parse(&bytes) else { continue };
            if let Some(manifest) = or_world_gta5::Manifest::new(&root) {
                tables.add_manifest(manifest);
            }
        }
        said(format!(
            "tables: {} texture-dictionary parent(s) from {} file(s), {} HD binding(s), {} map \
             group(s) with static bounds and {} interior(s) with bounds from {} manifest(s)",
            tables.relationships(),
            tables.relationship_files(),
            tables.hd_bindings(),
            tables.static_bound_groups(),
            tables.bounded_interiors().count(),
            tables.manifests()
        ));

        let mut archetypes = ArchetypeAssetIndex::new();
        let mut type_files = 0usize;
        for path in type_winners.values() {
            let Some(bytes) = mount.vfs.open(path) else {
                continue;
            };
            let Some(root) = parse(&bytes) else { continue };
            if let Some(types) = MapTypes::new(&root) {
                archetypes.add(types);
                type_files += 1;
            }
        }
        said(format!(
            "archetypes: {} from {type_files} `.ytyp` ({} redefined), {:.1} MiB held",
            archetypes.len(),
            archetypes.redefined(),
            archetypes.footprint() as f64 / (1024.0 * 1024.0)
        ));

        let started = std::time::Instant::now();
        let contents = ModelContents::read(&mount, &catalogue, &archetypes);
        said(format!(
            "bodies: {} `.ydd` and {} `.ydr` read ({} would not open) in {:.2}s",
            contents.dictionaries(),
            contents.drawables(),
            contents.unreadable(),
            started.elapsed().as_secs_f64()
        ));

        Self {
            mount,
            catalogue,
            archetypes,
            tables,
            contents,
            maps: map_winners.into_values().collect(),
            type_files,
        }
    }

    /// Every archetype the install's type files declare.
    pub(crate) fn archetypes(&self) -> &ArchetypeAssetIndex {
        &self.archetypes
    }

    /// The archetype a name, a `0x` hash or an asset path's stem names, else
    /// the first naming it as its asset.
    pub fn archetype_of(&self, subject: &str) -> Option<u32> {
        (self.archetypes).archetype_of(name_or_hash(subject_name(subject)))
    }

    /// What `subject` uses: a `.ymap`'s static bounds, each checked against the
    /// map's streaming box, else everything the archetype
    /// [`Self::archetype_of`] finds needs.
    pub fn uses(&self, subject: &str) -> Option<DependencySet> {
        self.uses_checked(subject, &self.map_boxes_of(subject, &|| false))
    }

    /// [`Self::uses`], a map's bounds checked against `boxes`, which
    /// [`Self::map_boxes_of`] reads apart so that a caller can read them off its UI thread.
    pub fn uses_checked(&self, subject: &str, boxes: &ModelContents) -> Option<DependencySet> {
        let resolver = Resolver::new(&self.catalogue, &self.archetypes, &self.tables);
        if is_map(subject) {
            return resolver
                .reading(boxes)
                .of_map(name_or_hash(subject_name(subject)));
        }
        resolver
            .reading(&self.contents)
            .of_archetype(self.archetype_of(subject)?)
    }

    /// The boxes a map's verdicts need, until `stop` says so: its streaming box
    /// and the header box of each `.ybn` its stem names, each read from the front of its file.
    /// None for any other subject, or for a map an imap group names the bounds of.
    pub fn map_boxes_of(&self, subject: &str, stop: &dyn Fn() -> bool) -> ModelContents {
        let mut boxes = ModelContents::new();
        if is_map(subject) {
            let name = name_or_hash(subject_name(subject));
            if let Some(logical) = (self.catalogue.find(name, Kind::MapData)).map(|l| &l.logical) {
                read_map_boxes(self, &mut boxes, stop, name, logical);
            }
        }
        boxes
    }

    /// Fold each map the engine would load into `index`, one tree at a time,
    /// until `enough` is told how many were read and says so; how many were.
    pub fn place(
        &self,
        index: &mut PlacementIndex,
        mut enough: impl FnMut(usize) -> bool,
    ) -> usize {
        let mut read = 0usize;
        for path in &self.maps {
            if enough(read) {
                break;
            }
            let Some(bytes) = self.mount.vfs.open(path) else {
                continue;
            };
            let Some(root) = parse(&bytes) else { continue };
            let Some(map) = MapData::new(&root) else {
                continue;
            };
            read += 1;
            index.add(&map);
        }
        read
    }
}

fn is_map(subject: &str) -> bool {
    subject.to_ascii_lowercase().ends_with(".ymap")
}

/// Read a map's streaming box and the boxes of the `.ybn` its stem names into
/// `contents`, for a map no imap group names, until `stop` says so.
fn read_map_boxes(
    index: &DependencyIndex,
    contents: &mut ModelContents,
    stop: &dyn Fn() -> bool,
    name: u32,
    logical: &str,
) {
    let DependencyIndex {
        mount,
        catalogue,
        tables,
        ..
    } = index;
    if !tables.static_bounds_of(name).is_empty() || catalogue.stem_bounds_of(name).is_empty() {
        return;
    }
    let Some(root) = mount.vfs.open(logical).and_then(|bytes| parse(&bytes)) else {
        return;
    };
    let Some(map) = MapData::new(&root) else {
        return;
    };
    if !contents.add_map(name, map) {
        return;
    }
    for &bound in catalogue.stem_bounds_of(name) {
        if stop() {
            return;
        }
        contents.read_bound(mount, catalogue, bound);
    }
}

fn cmd_of(args: &[String]) -> ExitCode {
    let (Some(game), Some(&target)) = (flag_value(args, "--game"), subjects(args).first()) else {
        eprintln!("Error: depends of --game <dir> <archetype-name|0xHASH|asset-path|map.ymap>");
        return ExitCode::from(2);
    };
    let loaded = match load(Path::new(game), flag_value(args, "--keys")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let a_map = is_map(target);
    let name = name_or_hash(subject_name(target));
    let Some(set) = loaded.uses(target) else {
        if a_map {
            eprintln!("no map {name:#010X} in this install");
        } else {
            eprintln!(
                "no archetype {name:#010X} in {} type file(s)",
                loaded.type_files
            );
        }
        return ExitCode::FAILURE;
    };

    println!();
    println!("{target} ({name:#010X})");
    if a_map {
        println!(
            "  static bounds from the manifest's MapDataGroups for an imap group; for any \
             other map, the `<map>_N`, `hi@<map>_N` and `ma@<map>_N` `.ybn` by filename \
             convention, which no file states, each checked against the map's streaming box"
        );
    } else {
        println!(
            "  states assetType {:?}, install holds it as {:?}",
            set.stated_asset_type, set.found_asset_kind
        );
    }
    println!();
    let mut outside = 0usize;
    for edge in &set.edges {
        let (via, verdict) = match edge.via {
            Via::MapStemConvention { .. } => (
                "MapStemConvention".to_string(),
                or_depends::export::verdict(edge.via)
                    .map_or_else(String::new, |says| format!("  [convention; {says}]")),
            ),
            via => (format!("{via:?}"), String::new()),
        };
        let flagged = matches!(
            edge.via,
            Via::MapStemConvention {
                inside_streaming_box: Some(false)
            }
        );
        outside += usize::from(flagged);
        let mark = match (edge.is_resolved(), flagged) {
            (false, _) => "!",
            (true, true) => "~",
            (true, false) => " ",
        };
        let where_ = match &edge.found {
            or_depends::Found::File(path) => path.clone(),
            or_depends::Found::Entry(path) => format!("(an entry inside) {path}"),
            or_depends::Found::Missing => "— not in this install".to_string(),
        };
        println!(
            "{mark} {via:<28} {:<13} {:#010X}  {where_}{verdict}",
            format!("{:?}", edge.kind),
            edge.name,
        );
    }
    println!();
    println!("{} of {} edge(s) resolved", set.resolved(), set.edges.len());
    if outside > 0 {
        println!("{outside} convention bound(s) outside the map's streaming box, marked ~");
    }
    if set.unresolved() > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn cmd_placements(args: &[String]) -> ExitCode {
    let (Some(game), Some(&target)) = (flag_value(args, "--game"), subjects(args).first()) else {
        eprintln!("Error: depends placements --game <dir> <model-name|0xHASH>");
        return ExitCode::from(2);
    };
    let loaded = match load(Path::new(game), flag_value(args, "--keys")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let name = name_or_hash(target);

    // ⚠ Watched rather than complete. A full positional index over a retail
    // install is tens of megabytes for an answer about one model, and the
    // denominator — every placement walked — is kept either way.
    let mut index = PlacementIndex::watching([name]);
    let limit = limit_of(args);
    let read = loaded.place(&mut index, |read| limit.is_some_and(|n| read >= n));

    let hits = index.of(name);
    println!();
    println!(
        "{target} ({name:#010X}): {} placement(s) across {} map(s), \
         out of {} placement(s) in {read} `.ymap`",
        hits.len(),
        index.maps_placing(name).len(),
        index.seen()
    );
    for hit in hits.iter().take(40) {
        println!(
            "  map {:#010X}  {:?}  ({:.2}, {:.2}, {:.2})",
            hit.map, hit.placed, hit.position.x, hit.position.y, hit.position.z
        );
    }
    if hits.len() > 40 {
        println!("  … and {} more", hits.len() - 40);
    }
    ExitCode::SUCCESS
}

/// Resolve every archetype the install declares, and report the coverage.
fn cmd_census(args: &[String]) -> ExitCode {
    let Some(game) = flag_value(args, "--game") else {
        eprintln!("Error: depends census --game <dir>");
        return ExitCode::from(2);
    };
    let mut loaded = match load(Path::new(game), flag_value(args, "--keys")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let started = std::time::Instant::now();
    let maps: Vec<(u32, String)> = (loaded.maps.iter())
        .map(|path| {
            let stem = subject_name(path).to_ascii_lowercase();
            (or_io::string_hash(stem.as_bytes()), path.clone())
        })
        .collect();
    let mut boxes = std::mem::take(&mut loaded.contents);
    for (name, path) in &maps {
        read_map_boxes(&loaded, &mut boxes, &|| false, *name, path);
    }
    loaded.contents = boxes;
    eprintln!(
        "map boxes: {} `.ymap` scanned in {:.2}s",
        maps.len(),
        started.elapsed().as_secs_f64()
    );
    let resolver = Resolver::new(&loaded.catalogue, &loaded.archetypes, &loaded.tables)
        .reading(&loaded.contents);
    let mut census = Census::new();
    for name in loaded.archetypes.names() {
        match resolver.of_archetype(name) {
            Some(set) => census.record_probing(&set, Some(&loaded.catalogue)),
            None => census.miss(),
        }
    }

    println!();
    println!(
        "{} archetype(s) from {} `.ytyp`; {} edge(s) considered",
        census.asked,
        loaded.type_files,
        census.exercised()
    );
    println!(
        "  {} of {} resolved to a file this install holds",
        census.edges.resolved, census.edges.considered
    );
    println!(
        "  {} of {} archetypes resolved every edge",
        census.complete, census.answered
    );
    println!(
        "  {} of {} interior(s) the manifest names bounds for are archetypes declared here",
        (loaded.tables.bounded_interiors())
            .filter(|n| loaded.archetypes.get(*n).is_some())
            .count(),
        loaded.tables.bounded_interiors().count()
    );
    println!();
    println!("by what states the edge:");
    for (via, cover) in &census.by_via {
        println!(
            "  {:<30} {:>8} of {:>8}",
            format!("{via:?}"),
            cover.resolved,
            cover.considered
        );
    }
    println!();
    println!("by the family the edge points at:");
    for (kind, cover) in &census.by_kind {
        println!(
            "  {:<14} {:>8} of {:>8}",
            format!("{kind:?}"),
            cover.resolved,
            cover.considered
        );
    }
    println!();
    println!(
        "{} edge(s) resolved to an entry inside another file rather than to a file",
        census.as_entry
    );
    println!();
    println!("where an unresolved name does live, if anywhere:");
    if census.missing_found_as.is_empty() && census.missing_entirely.is_empty() {
        println!("  nothing unresolved");
    }
    for ((via, kind), count) in &census.missing_found_as {
        println!(
            "  {:<30} {count:>8}  held as a {kind:?}, so the family asked for is wrong",
            format!("{via:?}")
        );
    }
    for (via, count) in &census.missing_entirely {
        println!(
            "  {:<30} {count:>8}  nowhere in the install under any family",
            format!("{via:?}")
        );
    }
    println!();
    println!(
        "what a physicsDictionary names, over the same {} edge(s):",
        census.physics.total()
    );
    println!(
        "  {:>8}  a `.ybn` or `.ybd` of that name",
        census.physics.a_bound_file
    );
    println!(
        "  {:>8}  no such file, and the archetype's own `.ydr` carries a bound",
        census.physics.in_the_model
    );
    println!(
        "  {:>8}  no such file, and the archetype's asset is itself a `.ybn`",
        census.physics.the_assets_own_bound
    );
    println!(
        "  {:>8}  no such file, and it is the archetype's own assetName",
        census.physics.the_assets_own_name
    );
    println!(
        "  {:>8}  no such file, and it is the archetype's own name",
        census.physics.the_archetypes_own_name
    );
    println!(
        "  {:>8}  no such file, and it is the archetype's textureDictionary",
        census.physics.the_texture_dictionary
    );
    println!("  {:>8}  none of those", census.physics.neither);
    println!();
    println!("assetType against what the install holds:");
    println!("  {:>8}  agree", census.asset_type_agreed);
    println!("  {:>8}  disagree", census.asset_type_disagreed);
    println!(
        "  {:>8}  stored as a number, so nothing to compare (no schema snapshot)",
        census.asset_type_unnameable
    );
    println!(
        "  {:>8}  name no asset, so nothing was expected",
        census.asset_type_names_nothing
    );

    let (mut groups, mut by_stem, mut bare) = (0usize, 0usize, 0usize);
    let (mut inside, mut outside, mut unchecked) = (0usize, 0usize, 0usize);
    for (name, _) in &maps {
        let Some(set) = resolver.of_map(*name) else {
            continue;
        };
        if !loaded.tables.static_bounds_of(*name).is_empty() {
            groups += 1;
        } else if set.edges.is_empty() {
            bare += 1;
        } else {
            by_stem += 1;
        }
        for edge in &set.edges {
            match edge.via {
                Via::MapStemConvention {
                    inside_streaming_box: Some(true),
                } => inside += 1,
                Via::MapStemConvention {
                    inside_streaming_box: Some(false),
                } => outside += 1,
                Via::MapStemConvention {
                    inside_streaming_box: None,
                } => unchecked += 1,
                _ => {}
            }
        }
    }
    println!();
    println!("static bounds, over {} `.ymap`:", maps.len());
    println!("  {groups:>8}  imap group(s), whose bounds the manifest names");
    println!(
        "  {by_stem:>8}  other map(s) taking {} `.ybn` by filename convention:",
        inside + outside + unchecked
    );
    println!("  {inside:>8}    inside the map's streaming box");
    println!("  {outside:>8}    outside it, flagged");
    println!("  {unchecked:>8}    not cross-checked, a body did not read");
    println!("  {bare:>8}  other map(s) whose stem names no `.ybn`");

    if census.is_vacuous() {
        eprintln!("error: nothing was exercised — this run checked no edge at all");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// The texture names a sample of models ask for, and which tier answers.
///
/// ⚠ **Sampled, and the sample is a cap rather than a default to remove.**
/// This is the one subcommand that reads bodies: a model and every texture
/// dictionary behind it, both of them paged resources. Over 159,153
/// archetypes that is the shape of run that has already taken a machine down,
/// so the cap is stated and the denominator printed.
fn cmd_textures(args: &[String]) -> ExitCode {
    let Some(game) = flag_value(args, "--game") else {
        eprintln!("Error: depends textures --game <dir> [--sample <n>]");
        return ExitCode::from(2);
    };
    let sample: usize = flag_value(args, "--sample")
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    let loaded = match load(Path::new(game), flag_value(args, "--keys")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let resolver = Resolver::new(&loaded.catalogue, &loaded.archetypes, &loaded.tables)
        .reading(&loaded.contents);

    // ⚠ Bounded, because a dictionary read once is read by every archetype
    // naming it and an unbounded cache is the whole install's texture names.
    const CACHE_CAP: usize = 4096;
    let mut txd_names: BTreeMap<u32, Vec<String>> = BTreeMap::new();

    let mut sources = or_depends::TextureSources::default();
    let mut models_read = 0usize;
    let mut models_failed = 0usize;
    let mut models_without_shaders = 0usize;
    let mut considered = 0usize;
    let mut dictionaries_read = 0usize;

    for name in loaded.archetypes.names() {
        if models_read + models_failed >= sample {
            break;
        }
        let Some(set) = resolver.of_archetype(name) else {
            continue;
        };
        // Loose drawables and fragments only: an entry inside a `.ydd` is a
        // different read and its own measurement.
        let Some(asset) = set.asset_edge() else {
            continue;
        };
        let (or_world_gta5::Kind::Drawable | or_world_gta5::Kind::Fragment) = asset.kind else {
            continue;
        };
        let or_depends::Found::File(path) = &asset.found else {
            continue;
        };
        considered += 1;
        let Some(bytes) = loaded.mount.vfs.open(path) else {
            models_failed += 1;
            continue;
        };
        let drawable = match asset.kind {
            or_world_gta5::Kind::Drawable => {
                GtaDrawable::<Gen8>::from_bytes(&bytes).ok().map(|d| d.base)
            }
            _ => GtaFragType::<Gen8>::from_bytes(&bytes)
                .ok()
                .and_then(|f| f.base.common_drawable.map(|d| d.base)),
        };
        let Some(drawable) = drawable else {
            models_failed += 1;
            continue;
        };
        models_read += 1;

        let Some(group) = drawable.shader_group.as_deref() else {
            models_without_shaders += 1;
            continue;
        };
        let dictionary = group.texture_dictionary.as_deref();
        let embedded: Vec<String> = dictionary
            .map(|d| {
                (d.entries.iter().flatten())
                    .map(|t| t.name.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();

        // The dictionaries behind this model, in fall-through order.
        let mut tiers: Vec<(or_world_gta5::Via, u32)> = Vec::new();
        for edge in &set.edges {
            if edge.kind == or_world_gta5::Kind::TextureDict && edge.is_resolved() {
                tiers.push((edge.via, edge.name));
            }
        }
        for (_, hash) in &tiers {
            if txd_names.contains_key(hash) || txd_names.len() >= CACHE_CAP {
                continue;
            }
            let Some(located) = loaded
                .catalogue
                .find(*hash, or_world_gta5::Kind::TextureDict)
            else {
                continue;
            };
            let Some(bytes) = loaded.mount.vfs.open(&located.logical) else {
                continue;
            };
            let names = PgDictionary::<GrcTexture>::from_bytes(&bytes)
                .map(|d| {
                    (d.entries.iter().flatten())
                        .map(|t| t.name.to_ascii_lowercase())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            dictionaries_read += 1;
            txd_names.insert(*hash, names);
        }

        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        for shader in &group.shaders {
            for slot in shader.textures() {
                let Some(name) = group.slot_name(slot) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let key = name.to_ascii_lowercase();
                if seen.insert(key.clone(), ()).is_some() {
                    continue;
                }
                if embedded.contains(&key) {
                    sources.embedded += 1;
                    continue;
                }
                let tier = tiers.iter().find(|(_, hash)| {
                    txd_names
                        .get(hash)
                        .is_some_and(|names| names.contains(&key))
                });
                match tier.map(|(via, _)| via) {
                    Some(or_world_gta5::Via::ArchetypeTextureDictionary) => {
                        sources.archetype_dictionary += 1
                    }
                    Some(or_world_gta5::Via::TextureDictionaryParent) => {
                        sources.parent_dictionary += 1
                    }
                    Some(or_world_gta5::Via::ManifestHdTexture) => sources.manifest_hd += 1,
                    _ => sources.nowhere += 1,
                }
            }
        }
    }

    println!();
    println!(
        "{models_read} model(s) read of {considered} sampled ({models_failed} would not open, \
         {models_without_shaders} carry no shader group); {dictionaries_read} `.ytd` read"
    );
    println!(
        "{} distinct texture reference(s); where each is satisfied:",
        sources.total()
    );
    println!(
        "  {:>8}  the model's own embedded dictionary",
        sources.embedded
    );
    println!(
        "  {:>8}  the `.ytd` the archetype names",
        sources.archetype_dictionary
    );
    println!(
        "  {:>8}  a dictionary that one falls through to",
        sources.parent_dictionary
    );
    println!(
        "  {:>8}  the manifest's high-detail dictionary",
        sources.manifest_hd
    );
    println!("  {:>8}  no tier holds it", sources.nowhere);

    if sources.total() == 0 {
        eprintln!(
            "error: nothing was exercised — no model was read, so this says nothing \
             about texture resolution. No sampled model opened as a Legacy drawable or \
             fragment and named a texture."
        );
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Copy an asset and its whole dependency set into a folder, or nothing.
fn cmd_export(args: &[String]) -> ExitCode {
    let (Some(game), Some(out), Some(&target)) = (
        flag_value(args, "--game"),
        flag_value(args, "--out"),
        subjects(args).first(),
    ) else {
        eprintln!("Error: depends export --game <dir> --out <dir> <name|asset-path|0xHASH>");
        return ExitCode::from(2);
    };
    let loaded = match load(Path::new(game), flag_value(args, "--keys")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let asked = name_or_hash(subject_name(target));
    let Some(name) = loaded.archetype_of(target) else {
        eprintln!(
            "no archetype named {asked:#010X} or naming it as its asset in {} type file(s)",
            loaded.type_files
        );
        return ExitCode::FAILURE;
    };
    let resolver = Resolver::new(&loaded.catalogue, &loaded.archetypes, &loaded.tables)
        .reading(&loaded.contents);
    let Some(set) = resolver.of_archetype(name) else {
        eprintln!("no archetype {name:#010X}");
        return ExitCode::FAILURE;
    };
    match or_depends::export(&set, &loaded.mount, Path::new(out)) {
        Ok(files) => {
            println!();
            println!(
                "{target} ({name:#010X}): {} file(s) into {out}",
                files.len()
            );
            for file in &files {
                let why: Vec<String> = file.why.iter().map(|e| format!("{:?}", e.via)).collect();
                println!("  {:<40} {}", file.leaf, why.join(", "));
            }
            println!("  {:<40} why each file is there", or_depends::MANIFEST);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {target} ({name:#010X}) {e}");
            ExitCode::FAILURE
        }
    }
}

/// The ruling, per format. Needs no install.
fn cmd_rules() -> ExitCode {
    println!();
    println!("What a dependency resolver can derive, per format.");
    println!();
    let mut by_from: BTreeMap<String, Vec<&or_depends::Rule>> = BTreeMap::new();
    for rule in RULES {
        by_from
            .entry(format!(".{}", rule.from.extension()))
            .or_default()
            .push(rule);
    }
    for (from, rules) in &by_from {
        println!("{from}");
        for rule in rules {
            let verdict = match rule.derivable {
                Derivable::Yes => "yes".to_string(),
                Derivable::NeedsTheInstall => "yes, with the whole install".to_string(),
                Derivable::ByConvention => "by convention, not by reference".to_string(),
                Derivable::No(_) => "no".to_string(),
            };
            let stated = match rule.stated {
                Stated::InTheFile(f) => format!("in the file, `{f}`"),
                Stated::Elsewhere(f) => format!("elsewhere, {f}"),
                Stated::ByFilename => "by filename".to_string(),
            };
            let to = match rule.to {
                or_world_gta5::Kind::Other => "(no file)".to_string(),
                kind => format!(".{}", kind.extension()),
            };
            println!("  -> {to:<9} {verdict}");
            println!("     {} — stated {stated}", rule.what);
            if let Derivable::No(reason) = rule.derivable {
                println!("     ⚠ {reason}");
            }
        }
        println!();
    }
    ExitCode::SUCCESS
}

fn print_usage() {
    eprintln!("What an asset needs, and every place a model is put.");
    eprintln!();
    eprintln!("Usage: openrage-cli depends <subcommand> [options]");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  of --game <dir> <name>          Every file the install would load for");
    eprintln!("                                  one archetype, and what it could not find;");
    eprintln!("                                  given `<map>.ymap`, the map's static bounds,");
    eprintln!(
        "                                  by filename convention where no group names them."
    );
    eprintln!("  placements --game <dir> <name>  Every map that puts that model in the world.");
    eprintln!("  census --game <dir>             Resolve every archetype the install declares");
    eprintln!("                                  and report the coverage with its denominator.");
    eprintln!("  textures --game <dir>           Read a sample of models and report which tier");
    eprintln!("                                  answers each shader texture reference.");
    eprintln!("  export --game <dir> --out <dir> <name|asset-path>");
    eprintln!("                                  Copy the asset and every file it needs, with");
    eprintln!("                                  a manifest saying why each is there.");
    eprintln!("  rules                         What each format states and what it cannot.");
    eprintln!("                                  Needs no install.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --game <path>  The install root.");
    eprintln!("  --keys <path>  Where to find the key cache.");
    eprintln!("  --limit <n>    Read at most this many maps, so a run's cost is stated.");
    eprintln!("  --sample <n>   How many models `textures` opens. Default 500.");
    eprintln!("  --out <path>   The folder `export` writes into.");
    eprintln!();
    eprintln!("Notes:");
    eprintln!("  A name may be given as text or as `0xHHHHHHHH`, because most archetype");
    eprintln!("  names in an install do not survive as strings.");
    eprintln!();
    eprintln!("  `of` exits non-zero when any dependency is missing from the install —");
    eprintln!("  a set that is nearly right is worse than none, so it is reported as a");
    eprintln!("  failure rather than a footnote. `export` refuses such a set and writes");
    eprintln!("  nothing, naming each piece the install lacks.");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠ A flag's value is not the subject. Reading it as one hashed the
    /// install path and reported "no archetype 0x028365C5" for every name
    /// asked about, which reads as a missing archetype rather than as a
    /// question that was never asked.
    #[test]
    fn a_flags_value_is_not_mistaken_for_the_subject() {
        let args: Vec<String> = ["--game", "D:/Games/GTAV", "prop_bench_01", "--limit", "40"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(subjects(&args), vec!["prop_bench_01"]);
        assert_eq!(flag_value(&args, "--game"), Some("D:/Games/GTAV"));

        let args: Vec<String> = ["--out", "D:/export", "prop_bench_01"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(subjects(&args), vec!["prop_bench_01"]);
    }

    /// An asset path asks for its stem; a bare name or hash is left alone.
    #[test]
    fn an_asset_path_asks_for_its_stem() {
        assert_eq!(subject_name("props/prop_bench_01.ydr"), "prop_bench_01");
        assert_eq!(subject_name("platform:\\x\\prop_a.yft"), "prop_a");
        assert_eq!(subject_name("prop_bench_01"), "prop_bench_01");
        assert_eq!(subject_name("0xCCAB9307"), "0xCCAB9307");
    }

    /// ⚠ A hash argument is the number, never the hash of the text `0x…`.
    #[test]
    fn a_hex_argument_is_read_as_the_number_it_spells() {
        assert_eq!(name_or_hash("0xCCAB9307"), 0xCCAB_9307);
        assert_eq!(
            name_or_hash("prop_bench_01"),
            or_io::string_hash(b"prop_bench_01")
        );
        // Nine digits is a name that happens to start `0x`.
        assert_eq!(
            name_or_hash("0xCCAB93070"),
            or_io::string_hash(b"0xCCAB93070")
        );
    }
}
