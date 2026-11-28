//! `world states`, `world delete-entity`, `world delete-map`, `world prefab`
//! and `world create --manifest`: the world edits that span files.
//!
//! ⚠ **Every edit here writes into `--out` and never into an install.** A
//! changed file lands under its logical path, a removed one is listed, and
//! packaging the result as a DLC is `openrage-cli dlc`'s business.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glam::Vec3;
use or_io::meta::binary::{parse_meta_binary_full, MetaLayout};
use or_io::meta::{Diagnostics, MetaInstance};
use or_world_gta5::execute::WorkingSet;
use or_world_gta5::map_data::MapData;
use or_world_gta5::prefab::{self, Catalogue, EntityShape};
use or_world_gta5::state::{weather_name, Gate, StateGroups, WorldState};
use or_world_gta5::{ArchetypeIndex, Manifest, MapTypes};

use super::world::{flag_value, mounted_winners, name_or_hash};

fn read_bytes(bytes: &[u8]) -> Result<(MetaInstance, MetaLayout), String> {
    let mut diags = Diagnostics::lenient();
    parse_meta_binary_full(crate::meta::schema(), bytes, &mut diags).map_err(|e| e.to_string())
}

fn read_file(path: &Path) -> Result<(MetaInstance, MetaLayout), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    read_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Where a changed file lands under `out`: its logical path, mirrored.
///
/// ⚠ Not its leaf. Every manifest is `_manifest.ymf`, so a delete touching
/// three of them would write one file three times.
///
/// ⚠ A label can also be a file path, and pushing an absolute one onto
/// `out` replaces `out` — which would write over the file that was read. So
/// every component is taken apart and a drive, a root or a `..` is dropped.
fn out_path(out: &Path, label: &str) -> PathBuf {
    let mut path = out.to_path_buf();
    for part in label
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".." && *p != ".")
    {
        path.push(part.trim_end_matches(':'));
    }
    path
}

/// Write what an edit produced under `out`, mirroring each logical path.
fn write_out(out: &Path, written: &[(String, Vec<u8>)], removed: &[String]) -> ExitCode {
    for (label, bytes) in written {
        let path = out_path(out, label);
        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("Error: {}: {e}", dir.display());
                return ExitCode::FAILURE;
            }
        }
        if let Err(e) = std::fs::write(&path, bytes) {
            eprintln!("Error: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!(
            "  wrote   {} ({} bytes, from {label})",
            path.display(),
            bytes.len()
        );
    }
    for label in removed {
        println!("  removed {label}");
    }
    ExitCode::SUCCESS
}

/// Every manifest in the mount, by logical path, read.
fn manifests(game: &or_title::MountedGame) -> Vec<(String, MetaInstance, MetaLayout)> {
    let mut paths = Vec::new();
    game.for_each_file(|logical, _c, leaf| {
        if leaf.to_ascii_lowercase().ends_with(".ymf") {
            paths.push(logical.to_string());
        }
    });
    paths
        .into_iter()
        .filter_map(|p| {
            let bytes = game.vfs.open(&p)?;
            let (root, layout) = read_bytes(&bytes).ok()?;
            Manifest::new(&root)?;
            Some((p, root, layout))
        })
        .collect()
}

/// `world states --game <dir> [--hour H] [--weather W] [--request M]... [--toggle M]`
pub fn cmd_states(args: &[String]) -> ExitCode {
    let Some(game_root) = flag_value(args, "--game") else {
        eprintln!("Error: world states --game <dir> [--hour H --weather W --request <map>... --toggle <map>]");
        return ExitCode::from(2);
    };
    let (game, winners) = match mounted_winners(Path::new(game_root)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut groups = StateGroups::new();
    let mut read = 0usize;
    for (path, root, _) in manifests(&game) {
        if let Some(m) = Manifest::new(&root) {
            groups.add_from(m, &path);
            read += 1;
        }
    }
    let mut map_files = 0usize;
    for path in winners
        .iter()
        .filter(|p| p.to_ascii_lowercase().ends_with(".ymap"))
    {
        let Some(bytes) = game.vfs.open(path) else {
            continue;
        };
        if let Ok((root, _)) = read_bytes(&bytes) {
            if let Some(map) = MapData::new(&root) {
                groups.add_map(&map);
                map_files += 1;
            }
        }
    }
    let mut gates: BTreeMap<&str, usize> = BTreeMap::new();
    for name in groups.names() {
        *gates
            .entry(match groups.gate(name) {
                Some(Gate::Scripted) => "scripted",
                Some(Gate::Timed { .. }) => "timed",
                Some(Gate::Weather { .. }) => "weather",
                Some(Gate::TimedWeather { .. }) => "timed and weather",
                None => "none",
            })
            .or_default() += 1;
    }
    let held: Vec<u32> = groups.held_back().collect();
    println!(
        "{read} manifests declare {} world-state groups ({} restated by a later one); {map_files} map files read",
        groups.len(),
        groups.redeclared()
    );
    for (gate, n) in &gates {
        println!("  {n:>6}  {gate}");
    }
    println!(
        "  {} maps wait to be asked for (flags bit 0), {} of them named by no group",
        held.len(),
        held.iter().filter(|m| !groups.contains(**m)).count()
    );
    let problems = groups.check();
    println!(
        "  {} problems between the groups and the maps",
        problems.len()
    );
    for p in problems.iter().take(20) {
        println!("    {p}");
    }

    let hour: u8 = flag_value(args, "--hour")
        .and_then(|h| h.parse().ok())
        .unwrap_or(12);
    let weather = flag_value(args, "--weather").unwrap_or("EXTRASUNNY");
    let mut state = WorldState::at(hour, weather);
    for (i, a) in args.iter().enumerate() {
        if a == "--request" {
            if let Some(m) = args.get(i + 1) {
                state.requested.insert(name_or_hash(m));
            }
        }
    }
    let active = held
        .iter()
        .filter(|m| groups.is_active(**m, &state))
        .count();
    println!(
        "\nAt {hour:02}:00 under {weather}{}: {active} of {} held-back maps stream",
        weather_name(state.weather).map_or(" (an unnamed weather)", |_| ""),
        held.len()
    );
    if let Some(toggle) = flag_value(args, "--toggle") {
        let map = name_or_hash(toggle);
        let on = !state.requested.contains(&map);
        let delta = groups.toggle(&mut state, map, on);
        println!(
            "{} {map:#010X}: load {} map(s) and {} bounds, unload {} map(s) and {} bounds",
            if on { "Requesting" } else { "Releasing" },
            delta.load.len(),
            delta.bounds_load.len(),
            delta.unload.len(),
            delta.bounds_unload.len()
        );
        for m in delta.load.iter().chain(&delta.unload) {
            println!("    map    {m:#010X}");
        }
        for b in delta.bounds_load.iter().chain(&delta.bounds_unload) {
            println!("    bounds {b:#010X}");
        }
    }
    ExitCode::SUCCESS
}

/// Every archetype the install's type files declare.
fn types_index(game: &or_title::MountedGame, winners: &[String]) -> ArchetypeIndex {
    let mut index = ArchetypeIndex::new();
    for p in winners
        .iter()
        .filter(|p| p.to_ascii_lowercase().ends_with(".ytyp"))
    {
        if let Some((root, _)) = game.vfs.open(p).and_then(|b| read_bytes(&b).ok()) {
            if let Some(t) = MapTypes::new(&root) {
                index.add(t);
            }
        }
    }
    index
}

/// Recompute flags and extents on what changed, and say what happened.
fn refresh(ws: &mut WorkingSet, index: &ArchetypeIndex) -> Result<(), String> {
    let r = ws.refresh_derived(index).map_err(|e| e.to_string())?;
    if r.maps > 0 {
        println!(
            "Recomputed flags and extents on {} changed map(s): extents rewritten on {}, kept on {} \
             naming {} archetype(s) no type file declares",
            r.maps, r.extents_recomputed, r.extents_kept, r.unresolved
        );
    }
    Ok(())
}

/// Every map's parent and parts, read once.
struct Index {
    paths: BTreeMap<u32, Vec<String>>,
    parent: BTreeMap<u32, u32>,
}

impl Index {
    fn of(game: &or_title::MountedGame, winners: &[String]) -> Self {
        let mut paths: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        let mut parent = BTreeMap::new();
        for p in winners
            .iter()
            .filter(|p| p.to_ascii_lowercase().ends_with(".ymap"))
        {
            let Some(bytes) = game.vfs.open(p) else {
                continue;
            };
            let Ok((root, _)) = read_bytes(&bytes) else {
                continue;
            };
            let Some(map) = MapData::new(&root) else {
                continue;
            };
            paths.entry(map.name()).or_default().push(p.clone());
            if map.parent() != 0 {
                parent.insert(map.name(), map.parent());
            }
        }
        Self { paths, parent }
    }

    /// A map and every map below it in the parent chain.
    fn family(&self, root: u32) -> Vec<u32> {
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            let at = out[i];
            out.extend(
                self.parent
                    .iter()
                    .filter(|(_, p)| **p == at)
                    .map(|(c, _)| *c),
            );
            i += 1;
        }
        out
    }
}

fn add_maps(ws: &mut WorkingSet, game: &or_title::MountedGame, index: &Index, names: &[u32]) {
    for name in names {
        for path in index.paths.get(name).into_iter().flatten() {
            if let Some(bytes) = game.vfs.open(path) {
                if let Ok((root, layout)) = read_bytes(&bytes) {
                    ws.add(path.clone(), root, layout);
                }
            }
        }
    }
}

/// `world delete-entity --game <dir> --map <name> --index N [--subtree] --out <dir>`
pub fn cmd_delete_entity(args: &[String]) -> ExitCode {
    let (Some(game_root), Some(map), Some(index), Some(out)) = (
        flag_value(args, "--game"),
        flag_value(args, "--map"),
        flag_value(args, "--index").and_then(|i| i.parse::<usize>().ok()),
        flag_value(args, "--out"),
    ) else {
        eprintln!("Error: world delete-entity --game <dir> --map <name|0xHASH> --index <n> [--subtree] --out <dir>");
        eprintln!(
            "  Every map naming this one as parent is renumbered, so the install is required."
        );
        return ExitCode::from(2);
    };
    let (game, winners) = match mounted_winners(Path::new(game_root)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let index_all = Index::of(&game, &winners);
    let map = name_or_hash(map);
    if !index_all.paths.contains_key(&map) {
        eprintln!("Error: no map named {map:#010X} in the install");
        return ExitCode::FAILURE;
    }
    let mut ws = WorkingSet::new();
    add_maps(&mut ws, &game, &index_all, &index_all.family(map));

    let result = if args.iter().any(|a| a == "--subtree") {
        ws.delete_subtree(map, index)
            .map(|s| (s.entities, s.renumbered))
    } else {
        ws.delete_entity(map, index).map(|r| (1, r))
    };
    let (gone, moved) = match result {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Refusing: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = refresh(&mut ws, &types_index(&game, &winners)) {
        eprintln!("Refusing: {e}");
        return ExitCode::FAILURE;
    }
    let written = match ws.finish() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Refusing: {e}\n  Nothing was written.");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "Deleted {gone} entit{} from {map:#010X}; renumbered {} local and {} cross-file LOD link(s)",
        if gone == 1 { "y" } else { "ies" },
        moved.local,
        moved.cross_file
    );
    write_out(Path::new(out), &written, &ws.removed())
}

/// `world delete-map --game <dir> --map <name> --out <dir>`
pub fn cmd_delete_map(args: &[String]) -> ExitCode {
    let (Some(game_root), Some(map), Some(out)) = (
        flag_value(args, "--game"),
        flag_value(args, "--map"),
        flag_value(args, "--out"),
    ) else {
        eprintln!("Error: world delete-map --game <dir> --map <name|0xHASH> --out <dir>");
        eprintln!("  A map is its files, its manifest entries and its world-state group.");
        return ExitCode::from(2);
    };
    let (game, winners) = match mounted_winners(Path::new(game_root)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let index = Index::of(&game, &winners);
    let map = name_or_hash(map);
    let mut ws = WorkingSet::new();
    add_maps(&mut ws, &game, &index, &index.family(map));
    for (path, root, layout) in manifests(&game) {
        ws.add(path, root, layout);
    }
    if let Err(e) = ws.delete_map(map) {
        eprintln!("Refusing: {e}");
        return ExitCode::FAILURE;
    }
    let written = match ws.finish() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Refusing: {e}\n  Nothing was written.");
            return ExitCode::FAILURE;
        }
    };
    println!("Deleted map {map:#010X}");
    write_out(Path::new(out), &written, &ws.removed())
}

/// `world create --game <dir> --like <donor> --name <n> --manifest <logical.ymf> --out <dir>`
///
/// ⚠ The donor is read out of the install by its map name: the entry of the
/// new map is a copy of the entry a manifest of the install states for it.
pub fn cmd_create_executed(args: &[String]) -> ExitCode {
    let (Some(game_root), Some(donor), Some(name), Some(manifest), Some(out)) = (
        flag_value(args, "--game"),
        flag_value(args, "--like"),
        flag_value(args, "--name"),
        flag_value(args, "--manifest"),
        flag_value(args, "--out"),
    ) else {
        eprintln!(
            "Error: world create --game <dir> --like <donor-map> --name <new> --manifest <logical.ymf> --out <dir> [--parent <map>]"
        );
        return ExitCode::from(2);
    };
    let (game, winners) = match mounted_winners(Path::new(game_root)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let index = Index::of(&game, &winners);
    let donor = name_or_hash(donor);
    let Some(donor_path) = index.paths.get(&donor).and_then(|p| p.first()).cloned() else {
        eprintln!("Error: no map named {donor:#010X} in the install");
        return ExitCode::FAILURE;
    };
    let Some((donor_root, donor_layout)) =
        game.vfs.open(&donor_path).and_then(|b| read_bytes(&b).ok())
    else {
        eprintln!("Error: {donor_path} does not read");
        return ExitCode::FAILURE;
    };
    let name = name_or_hash(name);
    let parent = flag_value(args, "--parent").map(name_or_hash).unwrap_or(0);

    let types_index = types_index(&game, &winners);
    let types: BTreeSet<u32> = {
        let map = MapData::new(&donor_root).expect("a map");
        let derived = or_world_gta5::manifest::dependencies_of(&map, &types_index);
        let resident = or_world_gta5::manifest::resident_type_files();
        derived.types.difference(&resident).copied().collect()
    };

    let mut ws = WorkingSet::new();
    for (path, root, layout) in manifests(&game) {
        ws.add(path, root, layout);
    }
    if parent != 0 {
        add_maps(&mut ws, &game, &index, &[parent]);
    }
    let label = format!("{}.ymap", flag_value(args, "--name").unwrap_or("created"));
    if let Err(e) = ws.create_map(
        label,
        donor_root,
        donor_layout,
        name,
        parent,
        &types,
        manifest,
    ) {
        eprintln!("Refusing: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = refresh(&mut ws, &types_index) {
        eprintln!("Refusing: {e}");
        return ExitCode::FAILURE;
    }
    let written = match ws.finish() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Refusing: {e}\n  Nothing was written.");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "Created {name:#010X} from {donor:#010X}, stated in {manifest} with {} type file(s)",
        types.len()
    );
    write_out(Path::new(out), &written, &[])
}

fn parse_vec3(text: &str) -> Option<Vec3> {
    let parts: Vec<f32> = text
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    (parts.len() == 3).then(|| Vec3::new(parts[0], parts[1], parts[2]))
}

/// The prefabs filed under one title's directory, read.
fn open_catalogue(dir: &Path, title: &str) -> Result<Catalogue, String> {
    let mut catalogue = Catalogue::new(title);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(catalogue);
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ymap"))
        })
        .collect();
    files.sort();
    for path in files {
        let (root, _) = read_file(&path)?;
        let key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        catalogue
            .add(key, &root)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(catalogue)
}

/// `world prefab capture|place|list ...`
pub fn cmd_prefab(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("capture") => prefab_capture(&args[1..]),
        Some("place") => prefab_place(&args[1..]),
        Some("list") => prefab_list(&args[1..]),
        _ => {
            eprintln!("Usage: openrage-cli world prefab <capture|place|list> ...");
            eprintln!("  capture <map.ymap> --name <n> --catalogue <dir> --title <title>");
            eprintln!("          [--indices a,b,c | --all] [--origin x,y,z]");
            eprintln!(
                "  place   <target.ymap> <prefab.ymap> --at x,y,z [--heading deg] <out.ymap>"
            );
            eprintln!("  list    --catalogue <dir>");
            ExitCode::from(2)
        }
    }
}

fn prefab_capture(args: &[String]) -> ExitCode {
    let positional: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with('-') && (*i == 0 || !args[*i - 1].starts_with("--")))
        .map(|(_, a)| a)
        .collect();
    let (Some(input), Some(name), Some(dir), Some(title)) = (
        positional.first(),
        flag_value(args, "--name"),
        flag_value(args, "--catalogue"),
        flag_value(args, "--title"),
    ) else {
        eprintln!("Error: world prefab capture <map.ymap> --name <n> --catalogue <dir> --title <title> [--indices a,b | --all] [--origin x,y,z]");
        return ExitCode::from(2);
    };
    let (root, layout) = match read_file(Path::new(input.as_str())) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let indices: Vec<usize> = match flag_value(args, "--indices") {
        Some(list) => list
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect(),
        None if args.iter().any(|a| a == "--all") => prefab::capturable(&root),
        None => {
            eprintln!("Error: name the entities with --indices a,b,c, or take every capturable one with --all");
            return ExitCode::from(2);
        }
    };
    let origin = match flag_value(args, "--origin").map(parse_vec3) {
        Some(Some(v)) => v,
        Some(None) => {
            eprintln!("Error: --origin takes x,y,z");
            return ExitCode::from(2);
        }
        None => MapData::new(&root)
            .and_then(|m| indices.first().and_then(|i| m.entities().nth(*i)))
            .map(|e| e.position())
            .unwrap_or(Vec3::ZERO),
    };
    let captured =
        match prefab::capture(&root, &indices, or_io::string_hash(name.as_bytes()), origin) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Refusing: {e}");
                return ExitCode::FAILURE;
            }
        };
    let title_dir = Path::new(dir).join(title);
    let mut catalogue = match open_catalogue(&title_dir, title) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: the catalogue: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = catalogue.add(name, &captured) {
        eprintln!("Refusing: {e}");
        return ExitCode::FAILURE;
    }
    let bytes = match or_world_gta5::write::encode(&captured, &layout) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Refusing: {e}\n  Nothing was written.");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&title_dir) {
        eprintln!("Error: {}: {e}", title_dir.display());
        return ExitCode::FAILURE;
    }
    let path = title_dir.join(format!("{name}.ymap"));
    if let Err(e) = std::fs::write(&path, &bytes) {
        eprintln!("Error: {}: {e}", path.display());
        return ExitCode::FAILURE;
    }
    println!(
        "Captured {} entit{} as `{name}` ({} bytes) into {}; the `{title}` catalogue now holds {}",
        indices.len(),
        if indices.len() == 1 { "y" } else { "ies" },
        bytes.len(),
        path.display(),
        catalogue.len()
    );
    ExitCode::SUCCESS
}

fn prefab_place(args: &[String]) -> ExitCode {
    let positional: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with('-') && (*i == 0 || !args[*i - 1].starts_with("--")))
        .map(|(_, a)| a)
        .collect();
    let executed = flag_value(args, "--game").is_some();
    let (Some(target), Some(prefab_path), Some(at)) = (
        positional.first(),
        positional.get(1),
        flag_value(args, "--at").and_then(parse_vec3),
    ) else {
        eprintln!("Error: world prefab place <target.ymap> <prefab.ymap> <out.ymap> --at x,y,z [--heading deg]");
        eprintln!("   or: world prefab place <target.ymap> <prefab.ymap> --at x,y,z --game <dir> --out <dir> [--manifest-file <f.ymf>]...");
        return ExitCode::from(2);
    };
    let out = match positional.get(2) {
        Some(out) => out.as_str(),
        None if executed => "",
        None => {
            eprintln!(
                "Error: name the output file, or pass --game and --out to state its types too"
            );
            return ExitCode::from(2);
        }
    };
    let heading = flag_value(args, "--heading")
        .and_then(|h| h.parse::<f32>().ok())
        .unwrap_or(0.0)
        .to_radians();
    let (mut root, layout) = match read_file(Path::new(target.as_str())) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (prefab_root, _) = match read_file(Path::new(prefab_path.as_str())) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(game_root) = flag_value(args, "--game") {
        return place_executed(
            args,
            game_root,
            target,
            root,
            layout,
            &prefab_root,
            at,
            heading,
        );
    }
    let placed = match prefab::place(&mut root, &prefab_root, at, heading) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Refusing: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bytes = match or_world_gta5::write::encode(&root, &layout) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Refusing: {e}\n  Nothing was written.");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(out, &bytes) {
        eprintln!("Error: {out}: {e}");
        return ExitCode::FAILURE;
    }
    let archetypes = prefab::PrefabInfo::of(&prefab_root)
        .map(|i| i.archetypes.len())
        .unwrap_or(0);
    println!(
        "Placed {} entit{} at index {}; {} guid(s) reassigned; {out}: {} bytes",
        placed.entities,
        if placed.entities == 1 { "y" } else { "ies" },
        placed.base,
        placed.guids_reassigned,
        bytes.len()
    );
    println!(
        "  The map now places {archetypes} archetype(s) from the prefab; its manifest entry has to \
         state their type files (`world manifest --game` reports what it derives)."
    );
    ExitCode::SUCCESS
}

/// `prefab place --game <dir> [--manifest-file <f.ymf>]... --out <dir>`: the
/// placement, the manifest entry stating its type files, and the recomputed
/// flags and extents, judged together and written together.
#[allow(clippy::too_many_arguments)]
fn place_executed(
    args: &[String],
    game_root: &str,
    target: &str,
    root: MetaInstance,
    layout: MetaLayout,
    prefab_root: &MetaInstance,
    at: Vec3,
    heading: f32,
) -> ExitCode {
    let Some(out) = flag_value(args, "--out") else {
        eprintln!("Error: with --game, name the output directory with --out <dir>");
        return ExitCode::from(2);
    };
    let (game, winners) = match mounted_winners(Path::new(game_root)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let index = types_index(&game, &winners);
    let Some(map) = MapData::new(&root).map(|m| m.name()) else {
        eprintln!("Error: {target} is not a CMapData");
        return ExitCode::FAILURE;
    };
    let mut ws = WorkingSet::new();
    ws.add(target.to_string(), root, layout);
    for (path, m, l) in manifests(&game) {
        ws.add(path, m, l);
    }
    for (i, a) in args.iter().enumerate() {
        if a == "--manifest-file" {
            let Some(path) = args.get(i + 1) else {
                continue;
            };
            match read_file(Path::new(path)) {
                Ok((m, l)) => ws.add(path.clone(), m, l),
                Err(e) => {
                    eprintln!("Error: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }
    let placed = match ws.place_prefab(map, prefab_root, at, heading) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Refusing: {e}");
            return ExitCode::FAILURE;
        }
    };
    let needed: BTreeSet<u32> = {
        let doc = ws.document(target).expect("added above");
        let derived = or_world_gta5::manifest::dependencies_of(
            &MapData::new(&doc.root).expect("a map"),
            &index,
        );
        if derived.unresolved > 0 {
            eprintln!(
                "Refusing: {} placed archetype(s) are declared by no type file in the install, so \
                 no manifest entry could name what the streamer has to load",
                derived.unresolved
            );
            return ExitCode::FAILURE;
        }
        let resident = or_world_gta5::manifest::resident_type_files();
        derived.types.difference(&resident).copied().collect()
    };
    let stated = match ws.state_types(map, &needed) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("Refusing: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = refresh(&mut ws, &index) {
        eprintln!("Refusing: {e}");
        return ExitCode::FAILURE;
    }
    let written = match ws.finish() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Refusing: {e}\n  Nothing was written.");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "Placed {} entit{} at index {} ({} guid(s) reassigned); {stated} type file(s) added to the \
         manifest entry",
        placed.entities,
        if placed.entities == 1 { "y" } else { "ies" },
        placed.base,
        placed.guids_reassigned
    );
    write_out(Path::new(out), &written, &[])
}

fn prefab_list(args: &[String]) -> ExitCode {
    let Some(dir) = flag_value(args, "--catalogue") else {
        eprintln!("Error: world prefab list --catalogue <dir>");
        return ExitCode::from(2);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        eprintln!("Error: {dir} is not a directory");
        return ExitCode::FAILURE;
    };
    let mut titles: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    titles.sort();
    let mut code = ExitCode::SUCCESS;
    for t in titles {
        let title = t
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        match open_catalogue(&t, &title) {
            Ok(c) => {
                println!(
                    "{title}: {} prefab(s), entities carrying {} members",
                    c.len(),
                    c.shape().map_or(0, EntityShape::len)
                );
                for (key, info) in c.iter() {
                    println!(
                        "  {key:<32} {:>5} entities  {:>4} archetypes  span {:.1} x {:.1} x {:.1} m",
                        info.entities,
                        info.archetypes.len(),
                        info.span.1.x - info.span.0.x,
                        info.span.1.y - info.span.0.y,
                        info.span.1.z - info.span.0.z
                    );
                }
            }
            Err(e) => {
                eprintln!("{title}: {e}");
                code = ExitCode::FAILURE;
            }
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_manifests_do_not_land_on_one_file() {
        let out = Path::new("out");
        let a = out_path(out, "update:/dlc_patch/a.rpf/_manifest.ymf");
        let b = out_path(out, "update:/dlc_patch/b.rpf/_manifest.ymf");
        assert_ne!(a, b);
        assert!(a.starts_with(out) && b.starts_with(out));
        for escape in [
            "x:/../../etc/passwd",
            r"D:\Work\source.ymap",
            "/abs/file.ymap",
        ] {
            let p = out_path(out, escape);
            assert!(p.starts_with(out), "{escape} -> {}", p.display());
            assert!(
                !p.components().any(|c| c == std::path::Component::ParentDir),
                "{escape}"
            );
        }
    }
}
