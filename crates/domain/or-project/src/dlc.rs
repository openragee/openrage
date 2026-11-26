//! The singleplayer DLC output: the plan of a `dlc.rpf`, the write of it and the read back.
//!
//! A pack is `setup2.xml`, `content.xml` and the files it registers, which is what the plan
//! lists, and a path that runs through a name ending in `.rpf` is a file inside that archive,
//! which the pack registers as one file. The types the game is told come from `data_file` lines
//! (the form a FiveM manifest states them in) and from the two extensions that settle one.
//!
//! ⚠ The archive is written by `or_title::dlc::build_pack`, the call `openrage-cli dlc create`
//! makes, so a plan that registers what a folder holds packs as that command packs it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::{NoOpCryptoProvider, RpfArchive, RpfBuilder, RpfEncryption, RpfNode};
use or_io::gta5::shaders::shader_defs::ParamRemap;
use or_title::dlc::{build_pack_with, file_type_for, DlcPackSpec, PackFile};

use crate::build::{check_file, moved, uncooked, BuildError, Loss, Verification};
use crate::container::Output;
use crate::contribution::{ArchiveContents, Contribution, ManifestEntry};
use crate::plan::{Budget, Origin, Plan, PlannedFile, Registration};
use crate::watch::Watch;

/// The file that is the pack, in the folder a build writes.
pub const PACK: &str = "dlc.rpf";

/// The name of the budget that counts what the fullest archive of a pack would hold.
pub const ENTRIES: &str = "archive_entries";

/// The most entries an archive holds before it is corrupt: the figure the clothing tools state, which no constant of this tree measures.
pub const MOST_ENTRIES: u64 = 65_535;

/// The two documents a pack is made of, which `build_pack` writes itself.
const SETUP: &str = "setup2.xml";
const CONTENT: &str = "content.xml";

/// The paths of the two documents in a pack.
pub(crate) const DOCUMENTS: [&str; 2] = [SETUP, CONTENT];

/// The type of a file that is an archive, whatever its name.
const ARCHIVE: &str = "RPF_FILE";

/// The folder of a pack every archive of maps is under, in the packs of both installs measured (`how_the_packs_of_an_install_carry_their_maps`).
pub const LEVEL_FOLDER: &str = "x64/levels/gta5";

/// What a pack states an archive of map files holds, as every pack of both installs that registers one states it.
pub const MAP_DATA: &str = "CONTENTS_DLC_MAP_DATA";

/// The path of the archive a file at `path` is inside, up to and including its name: the first folder that ends in `.rpf`.
pub(crate) fn archive_of(path: &str) -> Option<&str> {
    let mut end = 0;
    let mut names = path.split('/').peekable();
    while let Some(name) = names.next() {
        end += name.len();
        if names.peek().is_some() && name.to_ascii_lowercase().ends_with(".rpf") {
            return Some(&path[..end]);
        }
        end += 1;
    }
    None
}

/// Register what the plan holds, add the two documents a pack is, and say what cannot ship.
pub(crate) fn finish<'a>(
    owned: &mut Vec<(&'a str, PlannedFile)>,
    entries: &[(&'a str, &ManifestEntry)],
    archives: &[(&'a str, &ArchiveContents)],
    plan: &mut Plan,
) {
    let name = plan.resource.clone();
    plan.registrations = register(owned, entries, archives, &name, &mut plan.problems);
    let documents = documents_of(&name, &plan.registrations);
    for (path, text) in [(SETUP, documents.setup2), (CONTENT, documents.content)] {
        owned.push((
            "",
            PlannedFile {
                path: path.to_string(),
                origin: Origin::Document(text),
            },
        ));
    }
    plan.dlclist_entry = Some(documents.dlclist_entry);
}

/// The files of a pack, each with the type the game is told it is, and a problem for each that cannot be registered.
///
/// `name` is the pack's name, which is a device name and a folder name as well. `said_to_hold` is what the contents state the
/// archives of the pack hold.
pub(crate) fn register<'a>(
    owned: &[(&'a str, PlannedFile)],
    entries: &[(&'a str, &ManifestEntry)],
    said_to_hold: &[(&'a str, &ArchiveContents)],
    name: &str,
    problems: &mut Vec<String>,
) -> Vec<Registration> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        problems.push(format!(
            "`{name}` is not a pack name; it is a device name and a folder name, so it is [A-Za-z0-9_]"
        ));
    }

    let held: BTreeSet<&str> = owned.iter().map(|(_, file)| file.path.as_str()).collect();
    let stated_of = |archive: &str| {
        (said_to_hold.iter())
            .find(|(_, said)| said.path == archive)
            .map(|(_, said)| said.contents.as_str())
    };
    problems.extend(maps_in_a_pack(held.iter().copied(), stated_of));
    // The type each file is stated to have, by the content that states it.
    let mut stated: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    // The names the game asks a type file inside a nested archive by, each with its type.
    let mut asked: BTreeMap<&str, &str> = BTreeMap::new();
    for &(module, entry) in entries {
        if let Some(path) = loose_type_file(entry) {
            problems.push(needs_an_archive(path));
        }
        match (entry.key.as_str(), entry.values.as_slice()) {
            ("data_file", [file_type, path]) if is_behind(path, held.iter().copied()) => {
                asked.insert(path.as_str(), file_type.as_str());
            }
            ("data_file", [file_type, path]) => match stated.get(path.as_str()) {
                Some((held, by)) if *held != file_type.as_str() => problems.push(format!(
                    "content `{by}` says `{path}` is a {held} and content `{module}` says it is a {file_type}"
                )),
                Some(_) => {}
                None => {
                    stated.insert(path.as_str(), (file_type.as_str(), module));
                }
            },
            ("data_file", values) => problems.push(format!(
                "content `{module}` adds a `data_file` line with {} value(s), and it takes a type and a path",
                values.len()
            )),
            // The version, the game and the line that declares a map are said of a FiveM resource; a pack has no manifest to put them in.
            ("fx_version" | "game" | MAP_LINE, _) => {}
            (key, _) => problems.push(format!(
                "content `{module}` adds the manifest line `{key}`, which a singleplayer DLC cannot ship: it has no manifest and runs no script"
            )),
        }
    }

    let mut registered: BTreeMap<String, String> = BTreeMap::new();
    let mut archives: BTreeSet<String> = BTreeSet::new();
    let mut untyped: Vec<&str> = Vec::new();
    for (_, file) in owned.iter() {
        match archive_of(&file.path) {
            Some(archive) => {
                archives.insert(archive.to_string());
            }
            None => match stated
                .get(file.path.as_str())
                .map(|(file_type, _)| *file_type)
                .or_else(|| file_type_for(&file.path))
            {
                Some(file_type) => {
                    registered.insert(file.path.clone(), file_type.to_string());
                }
                // A map file that sits loose is said above, with the reason a type would not mend.
                None if is_map(&file.path) => {}
                None => untyped.push(&file.path),
            },
        }
    }
    if !untyped.is_empty() {
        problems.push(needs_a_type(&untyped));
    }
    for archive in archives {
        if registered.contains_key(&archive) {
            problems.push(format!(
                "`{archive}` is a file and an archive that other files go into"
            ));
        }
        registered.insert(archive, ARCHIVE.to_string());
    }
    let mut holds: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    for &(module, archive) in said_to_hold {
        let (path, contents) = (archive.path.as_str(), archive.contents.as_str());
        match holds.get(path) {
            // What a pack states is said of an archive it packs or of a name it asks a type file by.
            _ if registered.get(path).map(String::as_str) != Some(ARCHIVE)
                && !asked.contains_key(path) =>
            {
                problems.push(format!(
                    "content `{module}` states what the archive `{path}` holds, and the build puts no file in an archive of that name"
                ))
            }
            Some((held, by)) if *held != contents => problems.push(format!(
                "content `{by}` says the archive `{path}` holds {held} and content `{module}` says it holds {contents}"
            )),
            Some(_) => {}
            None => {
                holds.insert(path, (contents, module));
            }
        }
    }
    for (path, (_, module)) in &stated {
        if let Some(archive) = archive_of(path) {
            problems.push(format!(
                "content `{module}` states a type for `{path}`, which is inside the archive `{archive}`, and a pack registers the archive"
            ));
        } else if !held.contains(path) {
            problems.push(format!(
                "content `{module}` states a type for `{path}`, which the build does not hold"
            ));
        }
    }
    if registered.is_empty() {
        problems.push("a pack with no files mounts and registers nothing".to_string());
    }

    let mut registrations: Vec<Registration> = registered
        .into_iter()
        .map(|(path, file_type)| Registration {
            contents: (holds.get(path.as_str())).map_or_else(String::new, |(c, _)| c.to_string()),
            path,
            file_type,
            asked: false,
        })
        .collect();
    registrations.extend(asked.into_iter().map(|(path, file_type)| {
        Registration {
            path: path.to_string(),
            file_type: file_type.to_string(),
            contents: holds
                .get(path)
                .map_or_else(String::new, |(c, _)| c.to_string()),
            asked: true,
        }
    }));
    registrations.sort_by(|a, b| a.path.cmp(&b.path));
    registrations
}

/// The line of a FiveM manifest that declares the resource a map.
const MAP_LINE: &str = crate::resource::manifest::MAP_KEY;

fn is_map(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".ymap")
}

/// Whether `path` is the name the game asks a type file of `held` by: `props/a.ityp` for `props/<any>.rpf/a.ytyp`.
///
/// ⚠ The install registers a type file by an `.ityp` name of the folder its nested archive is in, and no file of that name exists (`where_the_install_keeps_its_map_files`).
fn is_behind<'a>(path: &str, mut held: impl Iterator<Item = &'a str>) -> bool {
    let (folder, leaf) = path.rsplit_once('/').unwrap_or(("", path));
    let Some(stem) = leaf
        .rsplit_once('.')
        .filter(|(_, ext)| ext.eq_ignore_ascii_case("ityp"))
        .map(|(stem, _)| stem)
    else {
        return false;
    };
    let file = format!("{stem}.ytyp");
    held.any(|held| {
        archive_of(held).is_some_and(|archive| {
            let within = archive.rsplit_once('/').map_or("", |(folder, _)| folder);
            within.eq_ignore_ascii_case(folder)
                && held[archive.len() + 1..].eq_ignore_ascii_case(&file)
        })
    })
}

/// Why the map files among `paths` cannot ship in a singleplayer DLC, one sentence for each reason; `stated_of` is what the pack states an archive holds.
///
/// ⚠ Each sentence is read by a person in the plan's problems and under the Builds window's matrix: no markup, and no name of a field of a file.
fn maps_in_a_pack<'a>(
    paths: impl Iterator<Item = &'a str>,
    stated_of: impl Fn(&str) -> Option<&'a str>,
) -> Vec<String> {
    let (mut loose, mut archives) = (Vec::new(), BTreeSet::new());
    for path in paths.filter(|path| is_map(path)) {
        match archive_of(path) {
            None => loose.push(path),
            Some(archive) => {
                archives.insert(archive);
            }
        }
    }
    let mut said = Vec::new();
    if let Some(first) = loose.first() {
        let more = match loose.len() - 1 {
            0 => String::new(),
            n => format!(" and {n} more"),
        };
        said.push(format!(
            "The map file {first}{more} would sit loose in the pack, and the game's own packs keep their maps inside an archive under {LEVEL_FOLDER} that the pack registers as map data: a singleplayer DLC ships a map only inside such an archive."
        ));
    }
    let level = format!("{LEVEL_FOLDER}/");
    for archive in archives {
        said.push(if !archive.to_ascii_lowercase().starts_with(&level) {
            format!(
                "The maps would ship inside the archive {archive}, and the game's own packs keep every archive of maps under {LEVEL_FOLDER}: a singleplayer DLC ships a map only from there."
            )
        } else if stated_of(archive) != Some(MAP_DATA) {
            format!(
                "The maps would ship inside the archive {archive}, which the pack does not state to hold map data, as every pack of the game states of an archive of maps: a singleplayer DLC cannot ship them."
            )
        } else {
            format!("The maps would ship inside the archive {archive}, {TURNED_ON}.")
        });
    }
    said
}

/// What still locks a pack whose maps are laid out and registered as the game's own are: the change set that turns them on.
///
/// ⚠ A pack built here turns every file on from one change set of the group that runs when the pack starts. Of the archives of
/// maps the packs of two installs register, none is turned on from that group: each is turned on from the map group of
/// multiplayer by a compat pack, from the group a level pack streams its update from, or from no group
/// (`how_the_packs_of_an_install_carry_their_maps`), and no setup states the map group of the story
/// (`which_groups_the_change_sets_of_an_install_belong_to`).
const TURNED_ON: &str = "laid out and registered as the game's own packs lay theirs out, but a build turns that archive on when the pack starts, and no pack of the two installs measured turns its maps on at start or for the story. A singleplayer DLC cannot ship maps until a pack that turns them on when it starts is seen to load in the game";

/// The pack of `plan` once the files at `gone` are not in it: a registration of a file nothing holds any more is taken out, and the two documents are written again; whether a registration is left.
///
/// `prefix` is where the pack's files sit in the plan's paths, empty for a pack and the pack's place in the game for an `.oiv` that installs one.
/// ⚠ With none left the registrations are empty and the documents are as they were, for the caller to decide what a pack with nothing in it comes to.
pub(crate) fn without(plan: &mut Plan, gone: &BTreeSet<String>, prefix: &str) -> bool {
    let held: BTreeSet<String> = (plan.files.iter())
        .filter(|f| !gone.contains(&f.path))
        .filter_map(|f| f.path.strip_prefix(prefix).map(str::to_string))
        .collect();
    // A registered archive is held by any file inside it, and a name the game asks by is held by the type file behind it.
    plan.registrations.retain(|r| {
        held.contains(&r.path)
            || (r.asked && is_behind(&r.path, held.iter().map(String::as_str)))
            || held.iter().any(|p| {
                p.strip_prefix(&r.path)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
    });
    if plan.registrations.is_empty() {
        return false;
    }
    let documents = documents_of(&plan.resource, &plan.registrations);
    for (path, text) in [(SETUP, documents.setup2), (CONTENT, documents.content)] {
        let path = format!("{prefix}{path}");
        if let Some(file) = plan.files.iter_mut().find(|f| f.path == path) {
            file.origin = Origin::Document(text);
        }
    }
    plan.dlclist_entry = Some(documents.dlclist_entry);
    true
}

/// The documents of the pack `name` that registers `registrations`.
pub(crate) fn documents_of(
    name: &str,
    registrations: &[Registration],
) -> or_title::dlc::PackDocuments {
    let mut spec = DlcPackSpec::new(name);
    spec.files = registrations
        .iter()
        .map(|r| pack_file(r, PathBuf::new()))
        .collect();
    spec.documents()
}

/// The file of a pack that `registration` is, read from `source`; a name the game asks by has no source.
fn pack_file(registration: &Registration, source: PathBuf) -> PackFile {
    let file = match registration.asked {
        true => PackFile::asked(&registration.path),
        false => PackFile::new(&registration.path, source),
    };
    file.with_file_type(&registration.file_type)
        .with_contents(&registration.contents)
}

/// What each archive of a pack holds, counted as the archive counts: a file is one entry, a nested archive is one, each folder is one, and the archive has a root.
///
/// The pack itself is the empty name. `paths` are the files of the pack, by the path each has in it.
pub(crate) fn entries_of(paths: &[&str]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    count_into(&mut counts, "", paths);
    counts
}

fn count_into(counts: &mut BTreeMap<String, u64>, held: &str, paths: &[&str]) {
    // Every name an entry has in this archive, so a folder shared by many files is counted once.
    let mut here: BTreeSet<&str> = BTreeSet::new();
    let mut inside: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for &path in paths {
        let own = match archive_of(path) {
            Some(archive) => {
                inside
                    .entry(archive)
                    .or_default()
                    .push(&path[archive.len() + 1..]);
                archive
            }
            None => path,
        };
        let mut name = own;
        loop {
            here.insert(name);
            match name.rsplit_once('/') {
                Some((parent, _)) => name = parent,
                None => break,
            }
        }
    }
    counts.insert(held.to_string(), here.len() as u64 + 1);
    for (archive, rest) in inside {
        let whole = if held.is_empty() {
            archive.to_string()
        } else {
            format!("{held}/{archive}")
        };
        count_into(counts, &whole, &rest);
    }
}

/// The budget of the entries the fullest archive of `paths` would hold, against the ceiling the build states or the most an archive holds, and a problem when it is over.
///
/// `whole` says the pack itself is an archive that counts; an `.oiv` adds to the game's folders, which this does not count.
pub(crate) fn count_entries(
    paths: &[&str],
    whole: bool,
    declared: &Output,
    budgets: &mut Vec<Budget>,
    problems: &mut Vec<String>,
) {
    let counts = entries_of(paths);
    let fullest = (counts.iter())
        .filter(|(archive, _)| whole || !archive.is_empty())
        .max_by_key(|(_, n)| **n);
    let Some((archive, used)) = fullest else {
        return;
    };
    let stated = declared.limits.get(ENTRIES).copied();
    let ceiling = stated.map_or(MOST_ENTRIES, |max| max.min(MOST_ENTRIES));
    budgets.retain(|b| b.name != ENTRIES);
    budgets.push(Budget {
        name: ENTRIES.to_string(),
        used: *used,
        ceiling: Some(ceiling),
    });
    budgets.sort_by(|a, b| a.name.cmp(&b.name));
    if *used > ceiling {
        let archive = if archive.is_empty() { PACK } else { archive };
        let who = match stated {
            Some(max) if max < MOST_ENTRIES => format!("that build `{}` allows", declared.id),
            _ => "an archive of the game holds".to_string(),
        };
        problems.push(format!(
            "the archive `{archive}` would hold {used} entries, over the {ceiling} {who}"
        ));
    }
}

/// Why a singleplayer DLC cannot ship what `contribution` hands over by itself, none when it can: a manifest line it has no manifest for, a map, what the content says locks it, a file the game is told no type for, or a type file that would sit outside every archive.
///
/// ⚠ The rules of [`finish`], read for one content: a type another content states for a file is not counted here.
pub(crate) fn refuses(contribution: &Contribution) -> Option<String> {
    for entry in &contribution.manifest {
        match (entry.key.as_str(), entry.values.as_slice()) {
            ("data_file" | "fx_version" | "game" | MAP_LINE, _) => {}
            (key, _) => {
                return Some(format!(
                    "it adds the manifest line `{key}`, which a singleplayer DLC cannot ship: it has no manifest and runs no script"
                ))
            }
        }
    }
    let files = contribution.files.iter().map(|file| file.path.as_str());
    let stated_of = |archive: &str| {
        (contribution.archives.iter())
            .find(|said| said.path == archive)
            .map(|said| said.contents.as_str())
    };
    if let Some(why) = maps_in_a_pack(files, stated_of).into_iter().next() {
        return Some(why);
    }
    if let Some(why) = contribution.locks.first() {
        return Some(why.clone());
    }
    let untyped = untyped(contribution);
    if !untyped.is_empty() {
        return Some(needs_a_type(&untyped));
    }
    (contribution.manifest.iter())
        .find_map(loose_type_file)
        .map(needs_an_archive)
}

/// The path a `data_file` line registers as a type file that sits in no archive of the pack: a `.ytyp` under `DLC_ITYP_REQUEST`.
///
/// ⚠ The install registers `.ityp` names, and 684 of the 685 are the `.ytyp` of a nested archive (`where_the_install_keeps_its_map_files`), so a `.ytyp` registered loose is a placement nobody measured.
fn loose_type_file(entry: &ManifestEntry) -> Option<&str> {
    match (entry.key.as_str(), entry.values.as_slice()) {
        ("data_file", [file_type, path])
            if file_type == "DLC_ITYP_REQUEST"
                && path.to_ascii_lowercase().ends_with(".ytyp")
                && archive_of(path).is_none() =>
        {
            Some(path)
        }
        _ => None,
    }
}

/// One plain sentence for a type file registered outside every archive.
///
/// ⚠ Read by a person in the plan's problems and under the Builds window's matrix: no markup, and no name of a field of a file.
fn needs_an_archive(path: &str) -> String {
    format!(
        "The type file {path} would sit loose in the pack, and the game's own packs keep theirs inside an archive the pack registers: a singleplayer DLC cannot ship it until a build writes that archive."
    )
}

/// The files `contribution` hands over that the game is told no type for: none in an archive, and none whose extension or a stated type settles one.
pub(crate) fn untyped(contribution: &Contribution) -> Vec<&str> {
    let stated: BTreeSet<&str> = (contribution.manifest.iter())
        .filter(|entry| entry.key == "data_file")
        .filter_map(|entry| match entry.values.as_slice() {
            [_, path] => Some(path.as_str()),
            _ => None,
        })
        .collect();
    (contribution.files.iter())
        .map(|file| file.path.as_str())
        .filter(|path| {
            archive_of(path).is_none()
                && !stated.contains(path)
                && file_type_for(path).is_none()
                && !is_map(path)
        })
        .collect()
}

/// One plain sentence for every file whose extension settles no type: how many, three of them by path, and where to give one.
///
/// ⚠ Read by a person in the plan's problems and under the Builds window's matrix: no markup, and no name of a field of a file.
fn needs_a_type(paths: &[&str]) -> String {
    let named: Vec<&str> = paths.iter().take(3).copied().collect();
    let more = paths.len().saturating_sub(3);
    let names = match (named.as_slice(), more) {
        (_, 1..) => format!("{} and {more} more", named.join(", ")),
        ([init @ .., last], 0) if !init.is_empty() => format!("{} and {last}", init.join(", ")),
        _ => named.join(", "),
    };
    // ⚠ The way out most people want, a FiveM resource that ships a script as it is, comes first; a type is for the few files the game registers.
    match paths.len() {
        1 => format!(
            "Build a FiveM resource to ship {names} as it is, or give it a type under File types: a singleplayer DLC needs one the game knows."
        ),
        n => format!(
            "Build a FiveM resource to ship these {n} files as they are ({names}), or give each a type under File types: a singleplayer DLC needs one the game knows."
        ),
    }
}

/// The pack the plan registers, each file's source found by `source`.
fn spec_of(plan: &Plan, source: impl Fn(&str) -> PathBuf) -> DlcPackSpec {
    let mut spec = DlcPackSpec::new(&plan.resource);
    spec.files = plan
        .registrations
        .iter()
        .map(|r| pack_file(r, source(&r.path)))
        .collect();
    spec
}

/// Write the plan as `staging/dlc.rpf`.
///
/// The files a pack registers must be files on the disk, so what a retarget made, each document a
/// content wrote and each nested archive are written to a hidden `.tree` folder in `staging` first,
/// which is removed after.
pub(crate) fn write(
    plan: &Plan,
    staging: &Path,
    remap: &ParamRemap,
    watch: &Watch,
) -> Result<Vec<Loss>, BuildError> {
    let scratch = staging.join(".tree");
    let mut losses = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut done = 0;
    for file in &plan.files {
        watch.go_on()?;
        watch.at(&file.path, done, plan.totals.bytes);
        done += file.bytes();
        match &file.origin {
            // The pack writes its own two documents, and every other one is a file like a copy.
            Origin::Document(_) if matches!(file.path.as_str(), SETUP | CONTENT) => {}
            Origin::Document(text) => {
                let at = scratch.join("docs").join(&file.path);
                put(&at, text.as_bytes())?;
                files.push((file.path.clone(), at));
            }
            Origin::Copy { from, bytes } => {
                let len = std::fs::metadata(from)
                    .map_err(|source| io(format!("reading {}", from.display()), source))?
                    .len();
                if len != *bytes {
                    return Err(BuildError::SourceChanged(from.clone()));
                }
                files.push((file.path.clone(), from.clone()));
            }
            Origin::Retarget { from, bytes } => {
                let (data, loss) = moved(plan, &file.path, from, *bytes, remap)?;
                let at = scratch.join("moved").join(&file.path);
                put(&at, &data)?;
                files.push((file.path.clone(), at));
                losses.extend(loss);
            }
            Origin::Generated(bytes) => {
                let at = scratch.join("generated").join(&file.path);
                put(&at, bytes)?;
                files.push((file.path.clone(), at));
            }
            Origin::Cook { .. } => return Err(uncooked(&file.path)),
        }
    }
    let mut counter = 0;
    let top = pack_level(files, &scratch, &mut counter, watch)?;
    let spec = spec_of(plan, |path| {
        top.iter()
            .find(|(held, _)| held == path)
            .map(|(_, source)| source.clone())
            .unwrap_or_default()
    });
    build_pack_with(
        &spec,
        &staging.join(PACK),
        RpfEncryption::Open,
        Arc::new(NoOpCryptoProvider::new()),
        &mut |_| !watch.cancel.is_set(),
    )
    .map_err(|e| match watch.cancel.is_set() {
        true => BuildError::Stopped,
        false => BuildError::Refused(vec![e.to_string()]),
    })?;
    std::fs::remove_dir_all(&scratch)
        .or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
        .map_err(|source| io(format!("removing {}", scratch.display()), source))?;
    watch.all(plan.totals.bytes);
    Ok(losses)
}

/// `files` with each run that goes through an archive's name packed into that archive, written under `scratch`.
fn pack_level(
    files: Vec<(String, PathBuf)>,
    scratch: &Path,
    counter: &mut usize,
    watch: &Watch,
) -> Result<Vec<(String, PathBuf)>, BuildError> {
    let mut direct = Vec::new();
    let mut inside: BTreeMap<String, Vec<(String, PathBuf)>> = BTreeMap::new();
    for (path, source) in files {
        match archive_of(&path) {
            Some(archive) => {
                let rest = path[archive.len() + 1..].to_string();
                inside
                    .entry(archive.to_string())
                    .or_default()
                    .push((rest, source));
            }
            None => direct.push((path, source)),
        }
    }
    for (archive, members) in inside {
        watch.go_on()?;
        let members = pack_level(members, scratch, counter, watch)?;
        let mut builder = RpfBuilder::default();
        for (path, source) in &members {
            builder
                .insert_file(path, source)
                .map_err(|e| BuildError::Refused(vec![format!("`{archive}`: {e}")]))?;
        }
        *counter += 1;
        let at = scratch.join("nested").join(format!("{counter}.rpf"));
        std::fs::create_dir_all(at.parent().unwrap_or(scratch))
            .map_err(|source| io(format!("creating {}", scratch.display()), source))?;
        builder
            .save(&at)
            .map_err(|e| BuildError::Refused(vec![format!("`{archive}`: {e}")]))?;
        direct.push((archive, at));
    }
    Ok(direct)
}

fn put(at: &Path, data: &[u8]) -> Result<(), BuildError> {
    if let Some(parent) = at.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| io(format!("creating {}", parent.display()), source))?;
    }
    std::fs::write(at, data).map_err(|source| io(format!("writing {}", at.display()), source))
}

fn io(what: String, source: std::io::Error) -> BuildError {
    BuildError::Io { what, source }
}

/// Read `folder/dlc.rpf` back against the plan, each file of it and of the archives inside it.
pub(crate) fn verify(plan: &Plan, folder: &Path) -> Verification {
    let mut found = Verification::default();
    let on_disk = std::fs::read_dir(folder)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0);
    let archive = match RpfArchive::open(folder.join(PACK)) {
        Ok(archive) => archive,
        Err(e) => {
            found.problems.push(format!("`{PACK}` does not open: {e}"));
            return found;
        }
    };
    if on_disk != 1 {
        found.problems.push(format!(
            "the folder holds {on_disk} entries, and a pack is `{PACK}` alone"
        ));
    }
    let entries = plan.files.iter().map(|f| (f.path.clone(), f)).collect();
    compare(&mut found, plan, &archive, entries, "");
    found
}

/// The files of `archive` against `entries`, the plan's files by their path inside it.
///
/// `held` is the path of the archive inside the pack, empty at its top.
fn compare(
    found: &mut Verification,
    plan: &Plan,
    archive: &RpfArchive,
    entries: Vec<(String, &PlannedFile)>,
    held: &str,
) {
    // ⚠ The archive lower-cases every name it stores, and the game hashes names without regard to case.
    let present: BTreeMap<String, &RpfNode> = archive
        .nodes
        .iter()
        .filter(|node| !node.is_dir())
        .map(|node| {
            (
                node.path()
                    .replace('\\', "/")
                    .trim_start_matches('/')
                    .to_lowercase(),
                node,
            )
        })
        .collect();
    let mut expected: BTreeSet<String> = BTreeSet::new();
    let mut inside: BTreeMap<String, Vec<(String, &PlannedFile)>> = BTreeMap::new();
    for (path, file) in entries {
        match archive_of(&path) {
            Some(nested) => {
                let rest = path[nested.len() + 1..].to_string();
                inside
                    .entry(nested.to_string())
                    .or_default()
                    .push((rest, file));
            }
            None => {
                expected.insert(path.to_lowercase());
                found.checked += 1;
                match present
                    .get(&path.to_lowercase())
                    .map(|node| extract(archive, node))
                {
                    None => found
                        .problems
                        .push(format!("`{}` is missing from the pack", file.path)),
                    Some(Err(why)) => found.problems.push(format!(
                        "`{}` cannot be read from the pack: {why}",
                        file.path
                    )),
                    Some(Ok(bytes)) => check_file(found, plan, file, &bytes),
                }
            }
        }
    }
    for (nested, entries) in inside {
        expected.insert(nested.to_lowercase());
        let whole = if held.is_empty() {
            nested.clone()
        } else {
            format!("{held}/{nested}")
        };
        let opened = present
            .get(&nested.to_lowercase())
            .ok_or_else(|| "it is missing from the pack".to_string())
            .and_then(|node| extract(archive, node))
            .and_then(|bytes| RpfArchive::from_bytes(bytes).map_err(|e| e.to_string()));
        match opened {
            Ok(inner) => compare(found, plan, &inner, entries, &whole),
            Err(why) => found
                .problems
                .push(format!("the archive `{whole}` does not open: {why}")),
        }
    }
    for path in present.keys().filter(|path| !expected.contains(*path)) {
        let whole = if held.is_empty() {
            path.clone()
        } else {
            format!("{held}/{path}")
        };
        found
            .problems
            .push(format!("`{whole}` is in the pack and not in the plan"));
    }
}

/// The bytes of one file of an archive, as the file it was added as.
fn extract(archive: &RpfArchive, node: &RpfNode) -> Result<Vec<u8>, String> {
    archive
        .read_entry(node.entry_index())
        .map_err(|e| e.to_string())
}
