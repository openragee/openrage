//! How the packs of a mount carry their maps, counted: the body the install suite runs over an install and a default suite runs
//! over a pack built by hand, with the pattern both are held to.
//!
//! ⚠ It prints counts and the names of groups, which are words of the format, and never a name of the game.

use std::collections::{BTreeMap, BTreeSet};

use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{Diagnostics, MetaInstance, SchemaLibrary, Value};
use or_maps::layout;
use or_title::dlc::{ContentXml, DataFileEntry, ListItem, SetupContentType, SetupData};
use or_title::MountedGame;
use or_world::map_types::{ArchetypeKind, MapTypes};
use or_world::{field, keys, Manifest};

/// The name the mount gives the title update, which is no pack of the DLC list.
const TITLE_UPDATE: &str = "TitleUpdate";

/// The device of the base game's own level files, which a pack's archive of the same path replaces.
const BASE: &str = "platform";

const ARCHIVE: &str = "RPF_FILE";
const MODELS: [&str; 4] = ["ydr", "ydd", "yft", "ytd"];

/// A path of a content document as a path inside its pack: no device, the platform spelled, lowercase.
fn inside_the_pack(filename: &str) -> String {
    let rest = filename.split_once(":/").map_or(filename, |(_, rest)| rest);
    rest.to_ascii_lowercase().replace("%platform%", "x64")
}

fn bump<K: Ord>(counts: &mut BTreeMap<K, usize>, key: K) {
    *counts.entry(key).or_default() += 1;
}

/// The content document of a pack with the documents it includes, as the mount reads them.
fn content_of(game: &MountedGame, device: &str, dat: &str) -> Option<ContentXml> {
    let dat = if dat.is_empty() { "content.xml" } else { dat };
    let mut content = ContentXml::parse(&game.vfs.open(&format!("{device}:/{dat}"))?).ok()?;
    let included: Vec<String> = (content.included_data_files.iter())
        .map(|item| item.value.clone())
        .chain(content.included_xml_files.iter().cloned())
        .collect();
    for path in included {
        if let Some(inner) = (game.vfs.open(&path)).and_then(|b| ContentXml::parse(&b).ok()) {
            content.data_files.extend(inner.data_files);
            content
                .content_change_sets
                .extend(inner.content_change_sets);
        }
    }
    Some(content)
}

/// What turns a file on: the group that names the change set, none when no group does, and whether the file is in the list of a map.
type TurnedOnBy = BTreeSet<(String, bool)>;

/// What each file a pack's change sets turn on is turned on by.
fn turned_on(content: &ContentXml, setup: &SetupData) -> BTreeMap<String, TurnedOnBy> {
    let mut by_file: BTreeMap<String, TurnedOnBy> = BTreeMap::new();
    for change_set in &content.content_change_sets {
        let named = |items: &[ListItem]| {
            (items.iter()).any(|item| item.trim().eq_ignore_ascii_case(change_set.name.trim()))
        };
        let mut groups: Vec<String> = (setup.content_change_set_groups.iter())
            .filter(|group| named(&group.content_change_sets))
            .map(|group| group.name_hash.trim().to_ascii_uppercase())
            .collect();
        if groups.is_empty() {
            groups.push("no group".to_string());
        }
        let own = change_set.files_to_enable.iter().map(|file| (file, false));
        let of_a_map = (change_set.map_change_set_data.iter())
            .flat_map(|map| map.files_to_enable.iter())
            .map(|file| (file, true));
        for (file, scoped) in own.chain(of_a_map) {
            let by = by_file.entry(inside_the_pack(file)).or_default();
            by.extend(groups.iter().map(|group| (group.clone(), scoped)));
        }
    }
    by_file
}

/// The kind of pack a setup states, as a figure spells it.
pub fn kind_of(kind: SetupContentType) -> String {
    format!("{kind:?}")
}

fn spelled(by: Option<&TurnedOnBy>) -> String {
    match by {
        None => "no change set".to_string(),
        Some(by) => (by.iter())
            .map(|(group, scoped)| match scoped {
                true => format!("{group}, in the list of a map"),
                false => format!("{group}, in the change set's own list"),
            })
            .collect::<Vec<_>>()
            .join(" and "),
    }
}

/// The archive a file of a pack is directly inside, and the file's own name.
fn archive_and_leaf(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

fn extension(leaf: &str) -> &str {
    leaf.rsplit_once('.').map_or("", |(_, extension)| extension)
}

fn folder_of(archive: &str) -> &str {
    archive.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// The names an entry lists under `key`.
fn listed(entry: &MetaInstance, key: u32) -> BTreeSet<u32> {
    (field::array_at(entry, key).iter())
        .filter_map(|value| match value {
            Value::Hash(name) => Some(name.hash),
            Value::UInt(name) => Some(*name as u32),
            _ => None,
        })
        .collect()
}

/// What a manifest names: the type files its entries for a map need, and the type files it has an entry for.
#[derive(Default)]
struct Stated {
    needed: BTreeSet<u32>,
    type_entries: BTreeSet<u32>,
}

fn stated_by(game: &MountedGame, logical: &str) -> Option<Stated> {
    let bytes = game.vfs.open(logical)?;
    let library = SchemaLibrary::default();
    let (root, _) = parse_meta_binary_full(&library, &bytes, &mut Diagnostics::lenient()).ok()?;
    let manifest = Manifest::new(&root)?;
    let mut stated = Stated::default();
    for (_, types) in manifest.imap_dependencies() {
        stated.needed.extend(types);
    }
    stated
        .needed
        .extend(manifest.flat_imap_dependencies().map(|(_, types)| types));
    for entry in field::structs_at(&root, field::name_hash("itypDependencies_2")) {
        stated
            .type_entries
            .extend(field::hash_at(entry, keys::ITYP_NAME));
        stated.needed.extend(listed(entry, keys::ITYP_DEP_ARRAY));
    }
    Some(stated)
}

/// Whether the type file at `logical` declares an interior, none when it does not read.
fn declares_an_interior(game: &MountedGame, logical: &str) -> Option<bool> {
    let bytes = game.vfs.open(logical)?;
    let library = or_io::meta::default_library();
    let (root, _) = parse_meta_binary_full(library, &bytes, &mut Diagnostics::lenient()).ok()?;
    let types = MapTypes::new(&root)?;
    let interior = types.archetypes().any(|a| a.kind() == ArchetypeKind::Mlo);
    Some(interior)
}

#[derive(Default)]
pub struct Figures {
    pub packs: usize,
    pub packs_with_maps: usize,
    /// The archives that hold a map file directly.
    pub map_archives: usize,
    pub registered: usize,
    pub as_archive: usize,
    pub as_map_data: usize,
    pub disabled: usize,
    pub persistent: usize,
    pub overlay: usize,
    /// The ones no registration names, and those of them at a path the base game holds an archive at too.
    pub unregistered: usize,
    pub unregistered_over_the_game: usize,
    pub under_the_level: usize,
    /// By how many folders below the level's folder the archive sits.
    pub depth: BTreeMap<usize, usize>,
    pub turned_on: BTreeMap<String, usize>,
    /// The same, with the kind of pack its setup states, as [`kind_of`] spells it.
    pub turned_on_in: BTreeMap<(String, String), usize>,
    pub with_types: usize,
    pub with_manifest: usize,
    pub with_models: usize,
    pub with_bounds: usize,
    pub map_data_without_maps: usize,
    /// The manifests of the packs, and those that are `_manifest.ymf` at the top of a nested archive.
    pub manifests: usize,
    pub manifests_atop_an_archive: usize,
    pub type_files: usize,
    pub type_files_with_maps: usize,
    pub type_files_with_maps_asked: usize,
    /// The type files beside maps whose archive holds a manifest, and those of them that manifest names as needed by a map.
    pub type_files_with_maps_and_manifest: usize,
    pub type_files_with_maps_named: usize,
    /// The type files a registration asks for, of every pack.
    pub asked: usize,
    pub asked_as_props: usize,
    pub asked_flags: BTreeMap<String, usize>,
    /// The same three flags, of the registration of the archive each type file asked for sits in.
    pub asked_archive_flags: BTreeMap<String, usize>,
    pub asked_archive_plain: usize,
    pub asked_archive_with_models: usize,
    pub asked_archive_with_maps: usize,
    pub asked_alone: usize,
    pub asked_under_props: usize,
    pub asked_turned_on: BTreeMap<String, usize>,
    pub asked_archive_turned_on: BTreeMap<String, usize>,
    pub asked_interiors: usize,
    pub asked_unread: usize,
    /// The type files asked for that a manifest of the mount names as needed by a map or by an interior.
    pub asked_named_by_a_manifest: usize,
    /// The type files of the packs that declare an interior: how many, and of those in an archive with a manifest, how many it has an entry for.
    pub interiors: usize,
    pub interiors_with_manifest: usize,
    pub interiors_stated: usize,
    pub title_update_maps: usize,
    pub title_update_maps_of_packs: usize,
}

pub fn measure(game: &MountedGame) -> Figures {
    let mut files: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut every_manifest: Vec<String> = Vec::new();
    game.for_each_file(|logical, _, leaf| {
        let (device, rest) = logical.split_once(":/").unwrap_or(("", logical));
        (files.entry(device.to_ascii_lowercase()).or_default()).push(rest.to_ascii_lowercase());
        if leaf.to_ascii_lowercase().ends_with(".ymf") {
            every_manifest.push(logical.to_string());
        }
    });
    // What any manifest of the mount names as needed, the base game's among them.
    let mut needed_anywhere: BTreeSet<u32> = BTreeSet::new();
    for logical in &every_manifest {
        needed_anywhere.extend(stated_by(game, logical).unwrap_or_default().needed);
    }
    // The archives of the base game's own levels, as a path of a pack spells them.
    let of_the_game: BTreeSet<String> = (files.get(BASE).into_iter().flatten())
        .map(|file| format!("x64/{}", archive_and_leaf(file).0))
        .collect();
    let level = format!("{}/", layout::PACK_FOLDER);
    let props = format!("{}/", layout::props_folder());
    let mut figures = Figures::default();
    for pack in &game.packs {
        let Some(held) = files.get(&pack.device_name.to_ascii_lowercase()) else {
            continue;
        };
        if pack.name_hash == TITLE_UPDATE {
            let maps = || held.iter().filter(|file| file.ends_with(".ymap"));
            figures.title_update_maps = maps().count();
            figures.title_update_maps_of_packs =
                maps().filter(|file| file.starts_with("dlc_patch/")).count();
            continue;
        }
        figures.packs += 1;
        let content = content_of(game, &pack.device_name, &pack.setup.dat_file)
            .unwrap_or_else(|| panic!("the content document of a pack does not read"));
        let entries: BTreeMap<String, &DataFileEntry> = (content.data_files.iter())
            .map(|entry| (inside_the_pack(&entry.filename), entry))
            .collect();
        let turned_on = turned_on(&content, &pack.setup);

        // What each archive of the pack holds directly, by extension, and how many type files.
        let mut holds: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut type_files_in: BTreeMap<&str, usize> = BTreeMap::new();
        for file in held {
            let (archive, leaf) = archive_and_leaf(file);
            holds.entry(archive).or_default().insert(extension(leaf));
            if leaf.ends_with(".ytyp") {
                bump(&mut type_files_in, archive);
            }
            if leaf.ends_with(".ymf") {
                figures.manifests += 1;
                figures.manifests_atop_an_archive +=
                    usize::from(leaf == "_manifest.ymf" && archive.ends_with(".rpf"));
            }
        }
        let has = |archive: &str, extensions: &[&str]| {
            (holds.get(archive))
                .is_some_and(|held| extensions.iter().any(|extension| held.contains(extension)))
        };
        let manifest_of = |archive: &str| {
            has(archive, &["ymf"])
                .then(|| format!("{}:/{archive}/_manifest.ymf", pack.device_name))
                .and_then(|logical| stated_by(game, &logical))
        };
        let map_archives: Vec<&str> = (holds.keys().copied())
            .filter(|archive| has(archive, &["ymap"]))
            .collect();
        figures.packs_with_maps += usize::from(!map_archives.is_empty());
        for archive in &map_archives {
            figures.map_archives += 1;
            if let Some(below) = archive.strip_prefix(&level) {
                figures.under_the_level += 1;
                bump(&mut figures.depth, below.matches('/').count());
            }
            figures.with_types += usize::from(has(archive, &["ytyp"]));
            figures.with_manifest += usize::from(has(archive, &["ymf"]));
            figures.with_models += usize::from(has(archive, &MODELS));
            figures.with_bounds += usize::from(has(archive, &["ybn"]));
            let Some(entry) = entries.get(*archive) else {
                figures.unregistered += 1;
                figures.unregistered_over_the_game += usize::from(of_the_game.contains(*archive));
                continue;
            };
            figures.registered += 1;
            figures.as_archive += usize::from(entry.file_type == ARCHIVE);
            figures.as_map_data += usize::from(entry.contents == layout::MAP_DATA);
            figures.disabled += usize::from(entry.disabled);
            figures.persistent += usize::from(entry.persistent);
            figures.overlay += usize::from(entry.overlay);
            let by = spelled(turned_on.get(*archive));
            bump(&mut figures.turned_on, by.clone());
            bump(
                &mut figures.turned_on_in,
                (by, kind_of(pack.setup.content_type)),
            );
        }
        figures.map_data_without_maps += (entries.iter())
            .filter(|(path, entry)| {
                entry.contents == layout::MAP_DATA && !map_archives.contains(&path.as_str())
            })
            .count();

        let mut manifests: BTreeMap<&str, Option<Stated>> = BTreeMap::new();
        for file in held.iter().filter(|file| file.ends_with(".ytyp")) {
            let (archive, leaf) = archive_and_leaf(file);
            let stem = leaf.trim_end_matches(".ytyp");
            let name = or_io::string_hash(stem.as_bytes());
            let asked_by = format!("{}/{stem}.ityp", folder_of(archive));
            let asked = entries.get(&asked_by);
            let logical = format!("{}:/{file}", pack.device_name);
            let interior = declares_an_interior(game, &logical);
            let manifest = manifests
                .entry(archive)
                .or_insert_with(|| manifest_of(archive));
            figures.type_files += 1;
            if map_archives.contains(&archive) {
                figures.type_files_with_maps += 1;
                figures.type_files_with_maps_asked += usize::from(asked.is_some());
                if let Some(stated) = manifest {
                    figures.type_files_with_maps_and_manifest += 1;
                    figures.type_files_with_maps_named +=
                        usize::from(stated.needed.contains(&name));
                }
            }
            if interior == Some(true) {
                figures.interiors += 1;
                if let Some(stated) = manifest {
                    figures.interiors_with_manifest += 1;
                    figures.interiors_stated += usize::from(stated.type_entries.contains(&name));
                }
            }
            let Some(asked) = asked else { continue };
            figures.asked += 1;
            figures.asked_as_props += usize::from(
                asked.file_type == layout::TYPE_FILE && asked.contents == layout::PROPS,
            );
            bump(
                &mut figures.asked_flags,
                format!(
                    "disabled {}, persistent {}, overlay {}",
                    asked.disabled, asked.persistent, asked.overlay
                ),
            );
            figures.asked_archive_plain += usize::from(
                (entries.get(archive))
                    .is_some_and(|e| e.file_type == ARCHIVE && e.contents.is_empty()),
            );
            if let Some(held) = entries.get(archive) {
                bump(
                    &mut figures.asked_archive_flags,
                    format!(
                        "disabled {}, persistent {}, overlay {}",
                        held.disabled, held.persistent, held.overlay
                    ),
                );
            }
            figures.asked_archive_with_models += usize::from(has(archive, &MODELS));
            figures.asked_archive_with_maps += usize::from(has(archive, &["ymap"]));
            figures.asked_alone += usize::from(type_files_in.get(archive) == Some(&1));
            figures.asked_under_props += usize::from(format!("{archive}/").starts_with(&props));
            bump(
                &mut figures.asked_turned_on,
                spelled(turned_on.get(&asked_by)),
            );
            bump(
                &mut figures.asked_archive_turned_on,
                spelled(turned_on.get(archive)),
            );
            figures.asked_named_by_a_manifest += usize::from(needed_anywhere.contains(&name));
            match interior {
                Some(interior) => figures.asked_interiors += usize::from(interior),
                None => figures.asked_unread += 1,
            }
        }
    }
    figures
}

pub fn report(figures: &Figures) {
    let f = figures;
    println!(
        "{} packs mounted beside the title update, {} of them hold a map file",
        f.packs, f.packs_with_maps
    );
    println!(
        "{} archives hold a map file; the content document of their pack registers {} of them and not {}, {} of which sit at a path the base game holds an archive at too",
        f.map_archives, f.registered, f.unregistered, f.unregistered_over_the_game
    );
    println!(
        "  of the {} registered: {} as {ARCHIVE}, {} stating {}, {} disabled, {} persistent, {} as an overlay",
        f.registered,
        f.as_archive,
        f.as_map_data,
        layout::MAP_DATA,
        f.disabled,
        f.persistent,
        f.overlay
    );
    println!(
        "  of the {}: {} under {}, by folders below it {:?}",
        f.map_archives,
        f.under_the_level,
        layout::PACK_FOLDER,
        f.depth
    );
    println!(
        "  of the {}: {} hold a type file, {} a manifest, {} a model or a texture dictionary, {} a bound",
        f.map_archives, f.with_types, f.with_manifest, f.with_models, f.with_bounds
    );
    println!("  what turns a registered one on, and the kind of pack its setup states:");
    for ((by, kind), n) in &f.turned_on_in {
        println!("    {n:5} {by}; {kind}");
    }
    println!(
        "{} registrations state {} of an archive that holds no map file",
        f.map_data_without_maps,
        layout::MAP_DATA
    );
    println!(
        "{} manifests in the packs, {} of them `_manifest.ymf` at the top of a nested archive",
        f.manifests, f.manifests_atop_an_archive
    );
    println!(
        "{} type files in the packs; {} sit in an archive that holds a map file, and a registration asks for {} of those; the archive of {} of them holds a manifest, which names {} as needed by a map",
        f.type_files,
        f.type_files_with_maps,
        f.type_files_with_maps_asked,
        f.type_files_with_maps_and_manifest,
        f.type_files_with_maps_named
    );
    println!(
        "{} type files are asked for by a registration: {} as {} stating {}; the archive of {} is registered {ARCHIVE} stating no contents, of {} holds a model or a texture dictionary, of {} a map file, of {} no other type file; {} sit under {}; {} declare an interior ({} did not read); a manifest names {} as needed",
        f.asked,
        f.asked_as_props,
        layout::TYPE_FILE,
        layout::PROPS,
        f.asked_archive_plain,
        f.asked_archive_with_models,
        f.asked_archive_with_maps,
        f.asked_alone,
        f.asked_under_props,
        layout::props_folder(),
        f.asked_interiors,
        f.asked_unread,
        f.asked_named_by_a_manifest
    );
    println!("  the flags of a request: {:?}", f.asked_flags);
    println!(
        "  the flags of the archive of a request: {:?}",
        f.asked_archive_flags
    );
    println!("  what turns a request on:");
    for (by, n) in &f.asked_turned_on {
        println!("    {n:5} {by}");
    }
    println!("  what turns the archive of a request on:");
    for (by, n) in &f.asked_archive_turned_on {
        println!("    {n:5} {by}");
    }
    println!(
        "{} type files of the packs declare an interior; the archive of {} holds a manifest, which has an entry for {}",
        f.interiors, f.interiors_with_manifest, f.interiors_stated
    );
    println!(
        "the title update holds {} map files, {} of them in the folders that patch a pack",
        f.title_update_maps, f.title_update_maps_of_packs
    );
}

/// The group whose change sets run when a pack starts, and the map group of the story.
const AT_START: &str = "GROUP_STARTUP";
const FOR_THE_STORY: &str = "GROUP_MAP_SP";
/// The map group of multiplayer, and the group a pack that updates a level streams its files from.
pub const FOR_MULTIPLAYER: &str = "GROUP_MAP,";
pub const AS_AN_UPDATE: &str = "GROUP_UPDATE_STREAMING,";

pub const MAPS_ON_AT_START: &str = "an archive of maps is turned on when its pack starts";

/// Where `figures` leave the pattern every pack of an install keeps, one sentence for each way.
pub fn departures(figures: &Figures) -> Vec<&'static str> {
    let f = figures;
    let mut found = Vec::new();
    let mut held = |holds: bool, otherwise: &'static str| {
        if !holds {
            found.push(otherwise);
        }
    };
    held(
        f.under_the_level == f.map_archives,
        "an archive of maps is outside the folder of the level",
    );
    held(
        [f.as_archive, f.as_map_data, f.disabled, f.persistent] == [f.registered; 4],
        "a registered archive of maps is not an archive stated as map data, disabled and persistent",
    );
    held(
        f.type_files_with_maps_asked == 0 && f.asked_archive_with_maps == 0,
        "a type file beside the maps is asked for by a registration",
    );
    held(
        [f.asked_as_props, f.asked_archive_plain] == [f.asked; 2],
        "a type file is asked for other than as props, or from an archive stated to hold something",
    );
    held(
        f.asked_archive_flags.values().sum::<usize>() == f.asked
            && (f.asked_archive_flags.keys())
                .all(|flags| flags.starts_with("disabled true, persistent true")),
        "the archive of a type file asked for is not registered disabled and persistent",
    );
    held(
        [f.asked_alone, f.asked_archive_with_models] == [f.asked; 2],
        "a type file asked for shares its archive with another, or with no model",
    );
    held(
        f.asked_interiors == 0,
        "a type file that declares an interior is asked for",
    );
    held(
        f.interiors_stated == f.interiors_with_manifest,
        "a type file that declares an interior has no entry in the manifest of its archive",
    );
    held(
        f.manifests_atop_an_archive == f.manifests,
        "a manifest is not at the top of a nested archive",
    );
    held(
        !f.turned_on.keys().any(|by| by.contains(AT_START)),
        MAPS_ON_AT_START,
    );
    held(
        !f.turned_on.keys().any(|by| by.contains(FOR_THE_STORY)),
        "an archive of maps is turned on by the map group of the story",
    );
    found
}
