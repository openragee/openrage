use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{Diagnostics, ErrorMode, MetaInstance};
use or_world::{
    archetypes::ArchetypeIndex, content_flags, extents, map_data::MapData, map_types::MapTypes,
    validate, MapGroup,
};

pub fn run(args: &[String]) -> ExitCode {
    let Some(sub) = args.first() else {
        print_usage();
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    // ⚠ `or_world`'s import, converge and encode read the installed default.
    crate::meta::schema();
    match sub.as_str() {
        "info" => cmd_info(rest),
        "check" => cmd_check(rest),
        "extents" => cmd_extents(rest),
        "manifest" => cmd_manifest(rest),
        "partition" => cmd_partition(rest),
        "create" if rest.iter().any(|a| a == "--manifest") => {
            super::world_edit::cmd_create_executed(rest)
        }
        "create" => cmd_create(rest),
        "new-map" => super::world_new::cmd_new_map(rest),
        "states" => super::world_edit::cmd_states(rest),
        "delete-entity" => super::world_edit::cmd_delete_entity(rest),
        "delete-map" => super::world_edit::cmd_delete_map(rest),
        "prefab" => super::world_edit::cmd_prefab(rest),
        "to-xml" => cmd_to_xml(rest),
        "from-xml" => cmd_from_xml(rest),
        "archetypes" => super::world_archetypes::run(rest),
        "index" => super::world_index::cmd_index(rest),
        "at" => super::world_index::cmd_at(rest),
        "ground" => super::world_ground::cmd_ground(rest),
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Error: unknown `world` subcommand '{other}'");
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn cmd_info(args: &[String]) -> ExitCode {
    let Some(path) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("Error: world info <file>");
        return ExitCode::from(2);
    };
    let path = PathBuf::from(path);
    let root = match read(&path, ErrorMode::Lenient) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("Error: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    println!("{}", path.display());
    if let Some(map) = MapData::new(&root) {
        report_map(&map);
    } else if let Some(types) = MapTypes::new(&root) {
        report_types(&types);
    } else {
        println!("  not a world file (root {})", root.type_name);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn report_map(map: &MapData<'_>) {
    let mut interiors = 0;
    let mut extensions = 0;
    let mut by_lod: BTreeMap<&str, usize> = BTreeMap::new();
    for entity in map.entities() {
        if entity.is_interior_instance() {
            interiors += 1;
        }
        extensions += entity.extension_count();
        *by_lod.entry(entity.lod_level().unwrap_or("?")).or_default() += 1;
    }

    println!("  root          CMapData");
    println!("  name          {:#010X}", map.name());
    println!("  parent        {:#010X}", map.parent());
    println!(
        "  flags         {:#010X}   content {:#010X}",
        map.flags(),
        map.content_flags()
    );
    println!(
        "  entities      {} ({interiors} interior instance(s), {extensions} extension(s))",
        map.entity_count()
    );
    for (lod, count) in &by_lod {
        println!("      {lod:<28} {count}");
    }
    println!(
        "  occluders     {} box, {} model",
        map.box_occluder_count(),
        map.occlude_model_count()
    );
    println!(
        "  generators    {} car, {} timecycle",
        map.car_generator_count(),
        map.time_cycle_modifier_count()
    );
    println!(
        "  instanced     {} grass batch(es), {} prop batch(es)",
        map.grass_batches().count(),
        map.prop_batch_count()
    );
    println!(
        "  lights        lod {}, distant {}",
        map.has_lod_lights(),
        map.has_distant_lod_lights()
    );
    if let Some((min, max)) = map.stored_entities_extents() {
        println!("  entities bb   {min:?} .. {max:?}");
    }
    if let Some((min, max)) = map.stored_streaming_extents() {
        println!("  streaming bb  {min:?} .. {max:?}");
    }

    let divergence = content_flags::Divergence::of(map);
    if !divergence.is_exact() {
        println!(
            "  flags differ  missing {:#010X}, extra {:#010X}",
            divergence.missing, divergence.extra
        );
    }
}

fn report_types(types: &MapTypes<'_>) {
    use or_world::map_types::ArchetypeKind;

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rooms = 0;
    let mut portals = 0;
    let mut sets = 0;
    let mut interior_entities = 0;
    for arch in types.archetypes() {
        let label = match arch.kind() {
            ArchetypeKind::Base => "base",
            ArchetypeKind::Time => "time",
            ArchetypeKind::Mlo => "interior",
        };
        *counts.entry(label).or_default() += 1;
        if let Some(interior) = arch.interior() {
            rooms += interior.room_count();
            portals += interior.portal_count();
            sets += interior.entity_set_count();
            interior_entities += interior.entity_count();
        }
    }

    println!("  root          CMapTypes");
    println!("  name          {:#010X}", types.name());
    println!("  archetypes    {}", types.archetype_count());
    for (kind, count) in &counts {
        println!("      {kind:<12} {count}");
    }
    if rooms + portals + sets > 0 {
        println!("  interiors     {rooms} room(s), {portals} portal(s), {sets} entity set(s)");
        println!("  contents      {interior_entities} entity(ies)");
    }
    println!("  dependencies  {}", types.dependency_count());
    println!("  composites    {}", types.composite_entity_type_count());
}

/// Where a run's world files come from.
///
/// ⚠ **Only one of these can answer a cross-file question.** A loose tree has
/// every copy of every map in it and no way to say which one the engine
/// loads; a mount has exactly one, chosen by mount order.
enum Source {
    Loose(Vec<PathBuf>),
    Mounted(or_title::MountedGame, Vec<String>),
}

impl Source {
    fn is_mounted(&self) -> bool {
        matches!(self, Source::Mounted(..))
    }

    fn len(&self) -> usize {
        match self {
            Source::Loose(files) => files.len(),
            Source::Mounted(_, paths) => paths.len(),
        }
    }

    fn truncate(&mut self, limit: usize) {
        match self {
            Source::Loose(files) => files.truncate(limit),
            Source::Mounted(_, paths) => paths.truncate(limit),
        }
    }

    fn labels(&self) -> Vec<String> {
        match self {
            Source::Loose(files) => files
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            Source::Mounted(_, paths) => paths.clone(),
        }
    }

    fn read_one(&self, label: &str) -> Result<MetaInstance, String> {
        match self {
            Source::Loose(_) => read(Path::new(label), ErrorMode::Lenient),
            Source::Mounted(game, _) => {
                let bytes = game
                    .vfs
                    .open(label)
                    .ok_or_else(|| format!("{label}: not in the mount"))?;
                parse(&bytes, ErrorMode::Lenient)
            }
        }
    }
}

/// How many offending files are listed rather than merely counted.
///
/// ⚠ A cap: a whole-install run found 938,000 problems under a wrong model
/// of one field.
const DETAIL_CAP: usize = 200;

fn cmd_check(args: &[String]) -> ExitCode {
    let json = args.iter().any(|a| a == "--json");
    let verbose = args.iter().any(|a| a == "--verbose");

    let mut source = match flag_value(args, "--game") {
        Some(game) => match mounted_winners(Path::new(game)) {
            Ok((game, paths)) => Source::Mounted(game, paths),
            Err(e) => {
                eprintln!("Error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            let Some(target) = args.iter().find(|a| !a.starts_with('-')) else {
                eprintln!("Error: world check <file-or-dir> | --game <dir>");
                return ExitCode::from(2);
            };
            let files = gather(Path::new(target), &["ymap", "ytyp"]);
            if files.is_empty() {
                eprintln!("Error: no .ymap or .ytyp under {target}");
                return ExitCode::FAILURE;
            }
            Source::Loose(files)
        }
    };
    if let Some(limit) = limit_of(args) {
        source.truncate(limit);
    }
    if source.is_mounted() {
        eprintln!("{} world file(s) the engine would load", source.len());
    }

    let mut checked = 0usize;
    let mut clean = 0usize;
    let mut by_problem: BTreeMap<String, usize> = BTreeMap::new();
    let mut offenders: Vec<(PathBuf, Vec<validate::Problem>)> = Vec::new();
    // The cross-file half: one group over everything read, so a whole staged
    // install is one group and a directory of a few maps is a partial one
    // that says so. It keeps a summary per map, not the tree.
    let mut group = MapGroup::new();

    // ⚠ One parsed tree at a time, and `root` is dropped at the end of every
    // iteration: retaining the trees for a 22,155-file run reached 38 GB and
    // was killed. Nothing below this line may outlive the loop body except a
    // count or a summary.
    for label in source.labels() {
        let Ok(root) = source.read_one(&label) else {
            continue;
        };
        let path = PathBuf::from(&label);
        let path = &path;
        checked += 1;
        let mut problems = Vec::new();
        if let Some(map) = MapData::new(&root) {
            problems.extend(validate::check(&map));
            group.add_from(&map, &label);
        }
        if let Some(types) = MapTypes::new(&root) {
            for arch in types.archetypes() {
                if let Some(interior) = arch.interior() {
                    problems.extend(validate::check_interior(&interior));
                }
            }
        }
        if problems.is_empty() {
            clean += 1;
        } else {
            for problem in &problems {
                *by_problem.entry(kind_of(problem).to_string()).or_default() += 1;
            }
            // Counted always, listed for the first `DETAIL_CAP` files only:
            // a corpus run reports a histogram, not a million sentences.
            if offenders.len() < DETAIL_CAP {
                offenders.push((path.clone(), problems));
            }
        }
    }

    let group_problems = group.check();
    let mut by_group_problem: BTreeMap<String, usize> = BTreeMap::new();
    for problem in &group_problems {
        *by_group_problem
            .entry(group_kind_of(problem).to_string())
            .or_default() += 1;
    }

    let files_with_problems: usize = by_problem.values().sum::<usize>().min(checked);
    if json {
        println!(
            "{{\"checked\":{checked},\"clean\":{clean},\"problems\":{files_with_problems},\
             \"mapsInGroup\":{},\"groupProblems\":{},\"groupBytes\":{}}}",
            group.len(),
            group_problems.len(),
            group.footprint()
        );
    } else {
        for (path, problems) in offenders.iter().take(if verbose { DETAIL_CAP } else { 40 }) {
            println!("{}", path.display());
            for problem in problems {
                println!("    {problem}");
            }
        }
        println!();
        println!("{clean} of {checked} clean, per file");
        for (kind, count) in &by_problem {
            println!("  {count:>8}  {kind}");
        }
        println!();
        println!(
            "{} map(s) in one group, {} entity(ies), summary {:.1} MiB",
            group.len(),
            group.entity_count(),
            group.footprint() as f64 / (1024.0 * 1024.0)
        );
        if group_problems.is_empty() {
            println!("  nothing to report across files");
        }
        for (kind, count) in &by_group_problem {
            println!("  {count:>8}  {kind}");
        }
        if verbose {
            for problem in group_problems.iter().take(40) {
                println!("    {problem}");
            }
        }

        let (inside, outside) = group.unresolved_links();
        let (declared, agreed) = group.declared_child_counts();
        println!();
        println!("cross-file LOD links:");
        println!("  {inside:>8}  land inside the parent map's own array");
        println!("  {outside:>8}  do not, which is refused");
        println!("  {agreed:>8}  of {declared} declared child counts agree with the group");
        let overstated = group
            .child_count_disagreements()
            .iter()
            .filter(|p| matches!(p, or_world::GroupProblem::ChildCountOverstated { .. }))
            .count();
        println!("  {overstated:>8}  overstate theirs, which is advisory rather than refused");

        let fit = group.index_space_fit();
        if fit.total > 0 {
            println!();
            println!("candidate index spaces for those {} links:", fit.total);
            for (label, hit) in [
                ("the parent's own entities array", fit.parent_own),
                ("the parent plus maps naming it", fit.parent_merged),
                ("the parent plus its descendants", fit.parent_chain),
                ("the child's own array", fit.own_array),
            ] {
                let pct = hit as f64 / fit.total as f64 * 100.0;
                println!("  {hit:>8}  {pct:>6.2}%  {label}");
            }
            println!(
                "  overflow past the parent's array: mean {:.1}, worst {}",
                fit.mean_overflow(),
                fit.worst_overflow
            );
            println!(
                "  {} link(s) point into {} parent map(s) that hold no entities at all",
                fit.outside_with_empty_parent, fit.empty_parents
            );
            println!(
                "  settled by one candidate: {}",
                if fit.is_settled() { "yes" } else { "no" }
            );
            for overflow in group.worst_overflows(4) {
                println!("    {overflow}");
            }
            for collision in group.collisions().iter().take(6) {
                println!("    {collision}");
            }
        }
    }

    if by_problem.is_empty() && group_problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn group_kind_of(problem: &or_world::GroupProblem) -> &'static str {
    use or_world::GroupProblem::*;
    match problem {
        ParentMapMissing { .. } => "parent map not in the group",
        ParentIndexWithoutParentMap { .. } => "cross-file LOD link on a map with no parent",
        ParentIndexOutOfRange { .. } => "cross-file LOD link past the parent's end",
        ChildCountUnderstated { .. } => "numChildren lower than the children naming it",
        ChildCountOverstated { .. } => "numChildren higher than the children naming it",
        DuplicateName { .. } => "two parts of one map both hold entities",
        ParentMapCycle { .. } => "parent-map cycle",
    }
}

fn kind_of(problem: &validate::Problem) -> &'static str {
    use validate::Problem::*;
    match problem {
        ParentOutOfRange { .. } => "LOD parent index past the end",
        ParentIsSelf { .. } => "entity is its own LOD parent",
        ParentCycle { .. } => "LOD parent cycle",
        ChildCountWrong { .. } => "numChildren disagrees with the links",
        AttachedObjectOutOfRange { .. } => "attached-object index past the end",
        EntitySetArraysDiffer { .. } => "entity set arrays differ in length",
        PortalRoomOutOfRange { .. } => "portal names a room that is not there",
        PortalNotAQuad { .. } => "portal is not four corners",
        LightArraysDiffer { .. } => "light arrays differ in length",
    }
}

fn cmd_extents(args: &[String]) -> ExitCode {
    let json = args.iter().any(|a| a == "--json");
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let Some(target) = positional.first() else {
        eprintln!("Error: world extents <file-or-dir> [--types <dir>]");
        return ExitCode::from(2);
    };
    let types_dir = flag_value(args, "--types")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(target));

    // ⚠ `--limit` caps the maps, never the archetypes. An index missing a
    // `.ytyp` reports the maps that used it as unresolved, which looks like
    // a finding and is a truncated read.
    let type_files = gather(&types_dir, &["ytyp"]);
    if type_files.is_empty() {
        eprintln!(
            "Error: no .ytyp under {} — extents need archetype bounds",
            types_dir.display()
        );
        return ExitCode::FAILURE;
    }
    // ⚠ One tree at a time here too. `ArchetypeIndex` copies the bounds it
    // needs, so the tree is free to go at the end of each iteration.
    let mut index = ArchetypeIndex::new();
    for path in &type_files {
        if let Ok(root) = read(path, ErrorMode::Lenient) {
            if let Some(types) = MapTypes::new(&root) {
                index.add(types);
            }
        }
    }
    eprintln!(
        "archetypes: {} from {} .ytyp ({} redefined)",
        index.len(),
        type_files.len(),
        index.redefined()
    );

    let mut maps = gather(Path::new(target), &["ymap"]);
    if let Some(limit) = limit_of(args) {
        maps.truncate(limit);
    }
    let mut complete = 0usize;
    let mut incomplete = 0usize;
    let mut buckets: BTreeMap<&str, usize> = BTreeMap::new();
    let mut worst: Option<(f32, PathBuf)> = None;

    for path in &maps {
        let Ok(root) = read(path, ErrorMode::Lenient) else {
            continue;
        };
        let Some(map) = MapData::new(&root) else {
            continue;
        };
        if map.entity_count() == 0 {
            continue;
        }
        let computed = extents::compute(&map, &index);
        if !computed.is_complete() {
            incomplete += 1;
            continue;
        }
        complete += 1;
        let Some(stored) = map.stored_entities_extents() else {
            continue;
        };
        let delta = extents::agreement(&computed.entities, stored);
        *buckets.entry(bucket(delta)).or_default() += 1;
        if worst.as_ref().is_none_or(|(w, _)| delta > *w) {
            worst = Some((delta, path.clone()));
        }
    }

    if json {
        println!(
            "{{\"maps\":{},\"complete\":{complete},\"incomplete\":{incomplete}}}",
            maps.len()
        );
    } else {
        println!();
        println!("{complete} map(s) with every archetype resolved, {incomplete} without");
        println!("agreement of the recomputed entity extents with the stored ones:");
        for (label, count) in &buckets {
            println!("  {count:>8}  {label}");
        }
        if let Some((delta, path)) = worst {
            println!("  worst {delta:.3} m at {}", path.display());
        }
    }
    ExitCode::SUCCESS
}

/// Compare what each map needs with what the manifests say it needs.
///
/// ⚠ Manifests first and maps second, because the stated half is small and
/// the derived half is not: a map's tree is dropped as soon as its
/// dependency set is folded in.
fn cmd_manifest(args: &[String]) -> ExitCode {
    let Some(game) = flag_value(args, "--game") else {
        eprintln!("Error: world manifest --game <dir>");
        eprintln!("  A map's dependencies are a fact about the install, not about one file.");
        return ExitCode::from(2);
    };
    let (mount, world_paths) = match mounted_winners(Path::new(game)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut manifest_paths: Vec<String> = Vec::new();
    mount.for_each_file(|logical, _container, leaf| {
        if leaf.to_ascii_lowercase().ends_with(".ymf") {
            manifest_paths.push(logical.to_string());
        }
    });

    let mut stated = or_world::StatedDependencies::new();
    let mut manifests_read = 0usize;
    for path in &manifest_paths {
        let Some(bytes) = mount.vfs.open(path) else {
            continue;
        };
        let Ok(root) = parse(&bytes, ErrorMode::Lenient) else {
            continue;
        };
        if let Some(manifest) = or_world::Manifest::new(&root) {
            stated.add(manifest);
            manifests_read += 1;
        }
    }
    eprintln!(
        "{manifests_read} of {} `.ymf` are manifests; they state {} map(s), {:.1} KiB held",
        manifest_paths.len(),
        stated.len(),
        stated.footprint() as f64 / 1024.0
    );

    // The archetype index, from the type files the engine would load.
    let mut index = ArchetypeIndex::new();
    let mut types_read = 0usize;
    for path in &world_paths {
        if !path.to_ascii_lowercase().ends_with(".ytyp") {
            continue;
        }
        let Some(bytes) = mount.vfs.open(path) else {
            continue;
        };
        let Ok(root) = parse(&bytes, ErrorMode::Lenient) else {
            continue;
        };
        if let Some(types) = MapTypes::new(&root) {
            index.add(types);
            types_read += 1;
        }
    }
    eprintln!(
        "{} archetype(s) from {types_read} `.ytyp`, {} with an owning type file",
        index.len(),
        index.len()
    );

    // ⚠ Folded by map name before comparing, not per file. A map is spread
    // over its metadata `.ymap` and one part per grass batch, and a manifest
    // entry is for the *map*; comparing per file charges a grass part with
    // the whole map's statement and reports 456 disagreements that are one
    // map each. Only the fold is kept, never a parsed tree.
    let limit = limit_of(args);
    let mut derived_by_name: BTreeMap<u32, or_world::manifest::Dependencies> = BTreeMap::new();
    let mut files = 0usize;
    for path in &world_paths {
        if !path.to_ascii_lowercase().ends_with(".ymap") {
            continue;
        }
        if limit.is_some_and(|n| files >= n) {
            break;
        }
        let Some(bytes) = mount.vfs.open(path) else {
            continue;
        };
        let Ok(root) = parse(&bytes, ErrorMode::Lenient) else {
            continue;
        };
        let Some(map) = MapData::new(&root) else {
            continue;
        };
        files += 1;
        let derived = or_world::manifest::dependencies_of(&map, &index);
        let slot = derived_by_name.entry(map.name()).or_default();
        slot.types.extend(derived.types);
        slot.unresolved += derived.unresolved;
    }
    eprintln!(
        "{files} `.ymap` folded into {} map(s)",
        derived_by_name.len()
    );

    let resident = or_world::manifest::resident_type_files();
    let mut agreement = or_world::manifest::Agreement::default();
    let mut unstated_types: BTreeMap<u32, u64> = BTreeMap::new();
    for (&name, derived) in &derived_by_name {
        let before = agreement.derived_extra;
        agreement.record_with(derived, stated.of(name), &resident);
        if agreement.derived_extra > before {
            for ityp in &derived.types {
                if !resident.contains(ityp) && !stated.of(name).is_some_and(|s| s.contains(ityp)) {
                    *unstated_types.entry(*ityp).or_default() += 1;
                }
            }
        }
    }

    println!();
    println!(
        "{} map(s) compared, treating {} type file(s) as always resident",
        agreement.maps,
        resident.len()
    );
    println!(
        "  {:>8}  derived list matches the manifests",
        agreement.exact
    );
    println!(
        "  {:>8}  manifests state a superset, which loads more than needed",
        agreement.stated_superset
    );
    println!(
        "  {:>8}  derived a type file the manifests do not state",
        agreement.derived_extra
    );
    println!("  {:>8}  no manifest mentions at all", agreement.unstated);
    println!(
        "  {:>8}  place an archetype no type file declares, so the derivation is a lower bound",
        agreement.incomplete
    );
    // ⚠ A manifest stating a map the install does not hold is the mirror
    // defect of a map no manifest states, and neither is visible from one
    // file. Reported rather than refused: a pack can legitimately state a
    // map another pack ships.
    let mut stated_absent: Vec<u32> = stated
        .names()
        .filter(|name| !derived_by_name.contains_key(name))
        .collect();
    println!(
        "  {:>8}  stated by a manifest and not held by the install",
        stated_absent.len()
    );
    stated_absent.truncate(4);
    for name in &stated_absent {
        println!("    {name:#010X}");
    }

    if !unstated_types.is_empty() {
        println!("  the type files those maps needed and no manifest stated:");
        let mut ranked: Vec<(u32, u64)> = unstated_types.into_iter().collect();
        ranked.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        for (ityp, maps) in ranked.iter().take(8) {
            println!("    {ityp:#010X}  needed by {maps} map(s)");
        }
    }

    ExitCode::SUCCESS
}

/// `world to-xml`: a map or type file as the XML it converges through.
///
/// ⚠ Withheld, as `meta to-xml` withholds, when the export would not come
/// back: re-importing it has to reach a fixed point without losing a value.
fn cmd_to_xml(args: &[String]) -> ExitCode {
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let Some(input) = positional.first() else {
        eprintln!("Error: world to-xml <file.ymap|file.ytyp> [out.xml] [--anyway]");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !args.iter().any(|a| a == "--anyway") {
        let why = match or_world::xml::converge(&bytes) {
            Ok(c) if c.values_moved => Some("re-importing its XML loses a value".to_string()),
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        if let Some(why) = why {
            eprintln!(
                "Error: {input}: {why}.\n  Nothing was written; pass --anyway to export it \
                 regardless."
            );
            return ExitCode::FAILURE;
        }
    }
    let mut diags = Diagnostics::new(ErrorMode::Lenient);
    let xml = parse_meta_binary_full(crate::meta::schema(), &bytes, &mut diags)
        .map_err(|e| e.to_string())
        .and_then(|(root, layout)| {
            or_world::xml::export(&root, &layout).map_err(|e| e.to_string())
        });
    let xml = match xml {
        Ok(x) => x,
        Err(e) => {
            eprintln!("Error: {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match positional.get(1) {
        Some(out) => match std::fs::write(out, xml.as_bytes()) {
            Ok(()) => {
                eprintln!("Wrote {out} ({} bytes)", xml.len());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: {out}: {e}");
                ExitCode::FAILURE
            }
        },
        None => {
            print!("{xml}");
            ExitCode::SUCCESS
        }
    }
}

/// `world from-xml`: an exported map or type file back to a container,
/// refused if the game would misread it.
fn cmd_from_xml(args: &[String]) -> ExitCode {
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let (Some(input), Some(out)) = (positional.first(), positional.get(1)) else {
        eprintln!("Error: world from-xml <in.xml> <out> [--as rbm|pso] [--like <donor>]");
        return ExitCode::from(2);
    };
    let xml = match std::fs::read(input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {input}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let donor = match flag_value(args, "--like").map(std::fs::read) {
        None => None,
        Some(Ok(bytes)) => {
            let mut diags = Diagnostics::new(ErrorMode::Lenient);
            match parse_meta_binary_full(crate::meta::schema(), &bytes, &mut diags) {
                Ok((_, layout)) => Some(layout),
                Err(e) => {
                    eprintln!("Error: the donor: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        Some(Err(e)) => {
            eprintln!("Error: the donor: {e}");
            return ExitCode::FAILURE;
        }
    };
    let format = match flag_value(args, "--as").map(|s| s.to_ascii_lowercase()) {
        Some(s) if s == "pso" => or_io::meta::BinaryFormat::Pso,
        Some(s) if s == "rbm" => or_io::meta::BinaryFormat::Rbm,
        Some(other) => {
            eprintln!("Error: --as takes rbm or pso, not '{other}'");
            return ExitCode::from(2);
        }
        None => donor
            .as_ref()
            .map(|d| d.format)
            .unwrap_or(or_io::meta::BinaryFormat::Rbm),
    };
    match or_world::xml::import(&xml, format, donor.as_ref()) {
        Ok(bytes) => match std::fs::write(out, &bytes) {
            Ok(()) => {
                eprintln!("Wrote {out} ({} bytes, {})", bytes.len(), format.label());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: {out}: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("Error: {input}: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Create a `.ymap` by cloning a donor and renaming it.
///
/// ⚠ A donor brings its own layout and its classes' signature words, so the
/// clone is an RBM file as the donor is; `world new-map` makes a map from
/// nothing, which is a PSO.
///
/// A created map has to appear in a manifest to exist for the game, so this
/// needs the mounted install and refuses whatever [`or_world::Plan`] refuses.
/// `--manifest` carries the plan out instead, writing the manifest entry
/// beside the map.
fn cmd_create(args: &[String]) -> ExitCode {
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let (Some(donor), Some(name), Some(out), Some(game)) = (
        flag_value(args, "--like"),
        flag_value(args, "--name"),
        positional.last().copied(),
        flag_value(args, "--game"),
    ) else {
        eprintln!(
            "Error: world create --game <dir> --like <donor.ymap> --name <map-name> <out.ymap>"
        );
        eprintln!("  A created map has to appear in a manifest to exist, so the set is required.");
        return ExitCode::from(2);
    };

    let bytes = match std::fs::read(donor) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {donor}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut diags = Diagnostics::new(ErrorMode::Lenient);
    let (mut root, layout) = match parse_meta_binary_full(crate::meta::schema(), &bytes, &mut diags)
    {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {donor}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if MapData::new(&root).is_none() {
        eprintln!("Error: {donor} is not a CMapData");
        return ExitCode::FAILURE;
    }

    let name_hash = name_or_hash(name);
    let parent_hash = flag_value(args, "--parent").map(name_or_hash).unwrap_or(0);

    // The set the new map would join, so the plan has something to judge.
    let (mount, world_paths) = match mounted_winners(Path::new(game)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut group = MapGroup::new();
    let mut index = ArchetypeIndex::new();
    for path in &world_paths {
        let Some(bytes) = mount.vfs.open(path) else {
            continue;
        };
        let Ok(tree) = parse(&bytes, ErrorMode::Lenient) else {
            continue;
        };
        if let Some(map) = MapData::new(&tree) {
            group.add_from(&map, path);
        } else if let Some(types) = MapTypes::new(&tree) {
            index.add(types);
        }
    }
    let mut stated = or_world::StatedDependencies::new();
    mount.for_each_file(|logical, _c, leaf| {
        if leaf.to_ascii_lowercase().ends_with(".ymf") {
            if let Some(bytes) = mount.vfs.open(logical) {
                if let Ok(tree) = parse(&bytes, ErrorMode::Lenient) {
                    if let Some(manifest) = or_world::Manifest::new(&tree) {
                        stated.add(manifest);
                    }
                }
            }
        }
    });

    // ⚠ Resident type files are excluded for the same reason the manifest
    // comparison excludes them: no manifest states `v_proc1`, so requiring
    // one to would refuse every map that places a blade of grass.
    let types = {
        let map = MapData::new(&root).expect("checked above");
        let derived = or_world::manifest::dependencies_of(&map, &index);
        let resident = or_world::manifest::resident_type_files();
        derived.types.difference(&resident).copied().collect()
    };
    let plan = or_world::Plan::new().with(or_world::Edit::CreateMap {
        name: name_hash,
        parent: parent_hash,
        types,
    });
    let problems = plan.check(&group, &stated);
    if !problems.is_empty() {
        eprintln!("Refusing: the set would be broken even if the file were not.");
        for problem in &problems {
            eprintln!("  {problem}");
        }
        return ExitCode::FAILURE;
    }

    set_hash(&mut root, or_world::keys::NAME, name_hash);
    set_hash(&mut root, or_world::keys::PARENT, parent_hash);

    let encoded = match or_world::write::encode(&root, &layout) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(out, &encoded) {
        eprintln!("Error: {out}: {e}");
        return ExitCode::FAILURE;
    }

    // ⚠ Read it back before claiming it was written. An encoder that
    // produced a document its own reader rejects has written a file the game
    // will not open either.
    match parse(&encoded, ErrorMode::Lenient) {
        Ok(back) => match MapData::new(&back) {
            Some(map) if map.name() == name_hash => {
                println!(
                    "{out}: {} bytes, name {name_hash:#010X}, parent {parent_hash:#010X}, \
                     {} entity(ies) carried from the donor",
                    encoded.len(),
                    map.entity_count()
                );
                ExitCode::SUCCESS
            }
            Some(map) => {
                eprintln!(
                    "Error: written file reads back with name {:#010X}, not {name_hash:#010X}",
                    map.name()
                );
                ExitCode::FAILURE
            }
            None => {
                eprintln!("Error: written file does not read back as a CMapData");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("Error: written file does not parse: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A map or archetype name, or the number it hashes to when the name is unrecoverable.
///
/// ⚠ Most map and archetype names in an install do not survive as strings, so
/// a tool that only took names could not address most of the world.
pub(crate) fn name_or_hash(text: &str) -> u32 {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        if hex.len() == 8 {
            if let Ok(hash) = u32::from_str_radix(hex, 16) {
                return hash;
            }
        }
    }
    or_io::string_hash(text.as_bytes())
}

fn set_hash(root: &mut MetaInstance, key: u32, hash: u32) {
    if let Some(slot) = or_world::field::get_mut(root, key) {
        match slot {
            or_io::meta::Value::Hash(h) => {
                h.hash = hash;
                h.raw.clear();
            }
            or_io::meta::Value::UInt(n) => *n = hash as u64,
            _ => {}
        }
    }
}

/// What the streaming split looks like, and which flag bits are authored.
///
/// ⚠ The second half is the one that matters for writing: a bit no
/// derivation produces is a bit recomputing the flags would clear.
fn cmd_partition(args: &[String]) -> ExitCode {
    let Some(game) = flag_value(args, "--game") else {
        eprintln!("Error: world partition --game <dir>");
        return ExitCode::from(2);
    };
    let (mount, world_paths) = match mounted_winners(Path::new(game)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let limit = limit_of(args);
    let mut by_suffix: BTreeMap<&'static str, (u64, u64, u64, u64)> = BTreeMap::new();
    let mut extra_bits: BTreeMap<u32, u64> = BTreeMap::new();
    let mut missing_bits: BTreeMap<u32, u64> = BTreeMap::new();
    let mut mixed = 0u64;
    let mut single = 0u64;
    let mut critical = 0u64;
    let mut read = 0usize;

    for path in &world_paths {
        if !path.to_ascii_lowercase().ends_with(".ymap") {
            continue;
        }
        if limit.is_some_and(|n| read >= n) {
            break;
        }
        let Some(bytes) = mount.vfs.open(path) else {
            continue;
        };
        let Ok(root) = parse(&bytes, ErrorMode::Lenient) else {
            continue;
        };
        let Some(map) = MapData::new(&root) else {
            continue;
        };
        read += 1;

        let leaf = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
        let suffix = if leaf.contains("_critical_") {
            "critical"
        } else if leaf.contains("_strm_") {
            "strm"
        } else if leaf.contains("_long_") {
            "long"
        } else if leaf.contains("_lod") {
            "lod"
        } else {
            "other"
        };

        let counts = or_world::partition::LayerCounts::of(&map);
        if counts.is_single_layer() {
            single += 1;
        } else {
            mixed += 1;
        }
        if or_world::partition::StreamingClass::of(&map)
            == or_world::partition::StreamingClass::Critical
        {
            critical += 1;
        }

        let slot = by_suffix.entry(suffix).or_default();
        slot.0 += 1;
        slot.1 += counts.hd as u64;
        slot.2 += counts.lod as u64;
        slot.3 += counts.slod as u64;

        let divergence = or_world::Divergence::of(&map);
        for bit in 0..32u32 {
            if divergence.extra & (1 << bit) != 0 {
                *extra_bits.entry(bit).or_default() += 1;
            }
            if divergence.missing & (1 << bit) != 0 {
                *missing_bits.entry(bit).or_default() += 1;
            }
        }
    }

    println!();
    println!("{read} map(s) read");
    println!("  suffix     maps        hd       lod      slod");
    for (suffix, (maps, hd, lod, slod)) in &by_suffix {
        println!("  {suffix:<10} {maps:>5} {hd:>9} {lod:>9} {slod:>9}");
    }
    println!();
    println!("  {single} single-layer, {mixed} mixed; {critical} declare the critical bit");
    println!();
    println!("contentFlags bits the file sets and the contents do not imply:");
    for (bit, maps) in &extra_bits {
        println!("  bit {bit:>2} ({:#06X})  on {maps} map(s)", 1u32 << bit);
    }
    println!("contentFlags bits the contents imply and the file does not set:");
    for (bit, maps) in &missing_bits {
        println!("  bit {bit:>2} ({:#06X})  on {maps} map(s)", 1u32 << bit);
    }

    ExitCode::SUCCESS
}

pub(crate) fn bucket(delta: f32) -> &'static str {
    match delta {
        0.0 => "exact",
        d if d < 0.01 => "under 1 cm",
        d if d < 1.0 => "under 1 m",
        d if d < 10.0 => "under 10 m",
        d if d < 100.0 => "under 100 m",
        _ => "100 m or more",
    }
}

/// `--limit N`, for a caller who wants to state how much they ran.
pub(crate) fn limit_of(args: &[String]) -> Option<usize> {
    flag_value(args, "--limit")?.parse().ok()
}

pub(crate) fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == flag {
            return it.next().map(String::as_str);
        }
        if let Some(rest) = arg.strip_prefix(&format!("{flag}=")) {
            return Some(rest);
        }
    }
    None
}

pub(crate) fn read(path: &Path, mode: ErrorMode) -> Result<MetaInstance, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    parse(&bytes, mode)
}

fn parse(bytes: &[u8], mode: ErrorMode) -> Result<MetaInstance, String> {
    let mut diags = Diagnostics::new(mode);
    parse_meta_binary_full(crate::meta::schema(), bytes, &mut diags)
        .map(|(inst, _)| inst)
        .map_err(|e| e.to_string())
}

/// The copy of every world file the engine would actually load.
///
/// ⚠ **A map is identified by its leaf name and there are many copies of
/// each.** A retail install ships 19,392 `.ymap` under 10,603 distinct names;
/// `update.rpf` and the DLC packs replace base-game maps wholesale, and a
/// replaced base map is frequently an **empty stub**. Grouping a flattened
/// crawl therefore resolves a LOD parent to a map with no entities, which is
/// what made 178,628 cross-file links look broken.
pub(crate) fn mounted_winners(
    game_root: &Path,
) -> Result<(or_title::MountedGame, Vec<String>), String> {
    let game = super::keys::mount_all(game_root, None)?;

    // Last mount wins, which is the engine's own rule. Keyed by leaf so the
    // winner is chosen before anything is parsed.
    let mut winners: BTreeMap<String, String> = BTreeMap::new();
    game.for_each_file(|logical, _container, leaf| {
        let lower = leaf.to_ascii_lowercase();
        if lower.ends_with(".ymap") || lower.ends_with(".ytyp") {
            winners.insert(lower, logical.to_string());
        }
    });
    Ok((game, winners.into_values().collect()))
}

pub(crate) fn gather(root: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        out.push(root.to_path_buf());
        return out;
    }
    walk(root, exts, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, exts, out);
        } else if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|e| exts.iter().any(|w| w.eq_ignore_ascii_case(e)))
        {
            out.push(path);
        }
    }
}

fn print_usage() {
    eprintln!("Read and check world data: `.ymap` placements and `.ytyp` archetypes");
    eprintln!();
    eprintln!("Usage: openrage-cli world <subcommand> [options]");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  info <file>              What one map or type file holds.");
    eprintln!("  check <file-or-dir>      Indices, counts and parallel arrays the engine");
    eprintln!("                           trusts and the reader does not enforce.");
    eprintln!("  extents <dir>            Recompute entity extents and report how far they");
    eprintln!("                           are from the ones the files store.");
    eprintln!("  manifest --game <dir>    Derive each map's `.ytyp` dependencies from what it");
    eprintln!("                           places and compare with `_manifest.ymf`.");
    eprintln!("  archetypes --like <donor.ytyp> --name <name> <drawables...> <out.ytyp>");
    eprintln!("                           Generate one archetype per drawable, bounds taken");
    eprintln!("                           from the drawable; --lod-dist and --txd state what");
    eprintln!("                           no drawable records. With no --like the type file is");
    eprintln!("                           made from nothing, as PSO, and may declare none; an");
    eprintln!("                           archetype made so states no assetType.");
    eprintln!("  to-xml <file> [out.xml]  Export as XML, withheld unless re-importing it");
    eprintln!("                           reaches a fixed point; --anyway overrides.");
    eprintln!("  from-xml <in.xml> <out>  Import, refusing a file the game would misread.");
    eprintln!("                           --as rbm|pso (RBM by default, as the game ships");
    eprintln!("                           world data); --like <file> keeps its signatures.");
    eprintln!("  states --game <dir>      World-state groups: what switches each one, and");
    eprintln!("                           what streams at --hour/--weather with --request;");
    eprintln!("                           --toggle <map> prints the load/unload delta.");
    eprintln!("  create --game <dir> --like <map> --name <new> --manifest <logical.ymf>");
    eprintln!("         --out <dir>       Clone a map and state it in a manifest, both written.");
    eprintln!("  new-map --name <map> [--entity <archetype> --at <x,y,z> --lod-dist <d>");
    eprintln!("          --types <ytyp-or-dir>] <out.ymap>");
    eprintln!("                           Make a map from nothing, with no donor and no schema");
    eprintln!("                           snapshot: empty, or placing one entity (--turn <x,y,z>");
    eprintln!("                           in degrees, --scale <xy>[,<z>], --flags <n>), with its");
    eprintln!("                           extents and content flags derived. Written as PSO;");
    eprintln!("                           --as rbm is refused, since an RBM class carries a");
    eprintln!("                           signature word that nothing derives.");
    eprintln!("  delete-entity --game <dir> --map <m> --index <n> [--subtree] --out <dir>");
    eprintln!("                           Remove an entity (with its LOD children under");
    eprintln!("                           --subtree), renumbering every link in every file.");
    eprintln!("  delete-map --game <dir> --map <m> --out <dir>");
    eprintln!("                           Remove a map, its manifest entries and its group.");
    eprintln!("  prefab capture|place|list");
    eprintln!("                           Prefab catalogues, one directory per title.");
    eprintln!("  index --game <dir>       Build the world index of an install, or refresh the");
    eprintln!("                           kept one: every map, type file, archetype and bound,");
    eprintln!("                           and which archives were read again. --kept <file>");
    eprintln!("                           names the file it is kept in; --fresh reads it all.");
    eprintln!("  at --game <dir> <x> <y> <z>   (or a place as pasted: x,y,z or vector3(x, y, z))");
    eprintln!("                           The maps whose streaming extents hold a position and");
    eprintln!("                           the bounds whose box does, from the world index.");
    eprintln!("  ground --game <dir> <x> <y> [<z>]   (or a place as pasted, as `at` takes one)");
    eprintln!("                           The ground under a position: its height, material");
    eprintln!("                           and bounds file, or none and the files looked in.");
    eprintln!("                           With no z, the highest ground of the column; with");
    eprintln!("                           one, from --rise <m> above it (1) down. --slope");
    eprintln!("                           <degrees> is the steepest ground (60). --level");
    eprintln!("                           <pack> and --mode <story|multiplayer> mount as");
    eprintln!("                           `dlc-list` does. Static map collision only: an");
    eprintln!("                           interior's is named and not loaded, and a placed");
    eprintln!("                           prop's is not read.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --types <dir>   Where to find `.ytyp` for archetype bounds. Defaults to");
    eprintln!("                  the target, which is right for a whole staged install.");
    eprintln!("  --limit <n>     Read at most this many maps, so a run's cost is stated");
    eprintln!("                  rather than discovered. Never caps the archetype index:");
    eprintln!("                  a partial one reports truncation as unresolved archetypes.");
    eprintln!("  --json          Machine-readable summary.");
    eprintln!("  --verbose       List up to 200 offending files rather than the first 40.");
    eprintln!();
    eprintln!("Notes:");
    eprintln!("  An entity's box lives in a `.ytyp`, usually in another archive, so");
    eprintln!("  `extents` needs both halves and reports what it could not resolve");
    eprintln!("  rather than guessing a size.");
    eprintln!();
    eprintln!("  Both subcommands hold one parsed file at a time. `check` keeps a");
    eprintln!("  per-map summary for its cross-file half and reports its size, because");
    eprintln!("  a whole install's value trees do not fit in memory.");
}
