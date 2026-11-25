//! Reading the project off the disk: its files, its source folders and the plan of each build.
//!
//! ⚠ **Runs on a worker.** A project's folder may hold thousands of files, so the table reads what
//! a scan left and never touches the disk while it draws.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use or_io::Generation;
use or_project::{
    build_root, find_sources, links::linked_sources, shadow_root, since_last_build, Container,
    Diff, ModuleKind, Plan, Presence, Source as Held,
};
use or_source::asset::{parse_folder_name, Source};
use or_source::{bounds, metadata, models, particles, paths, textures};
use or_wardrobe::Garment;

use super::build_cooker::describe_all;
use super::build_edits::{locks_of, Lock};
use super::output_sizes::of_output;
use super::wardrobe::{read_packs, Pack, PackRead};
use crate::state::document_io::read_file_generation;
use crate::state::plugins::extensions::{extend, pack_document, Extension, Said};
use crate::state::DocumentKind;

/// A source's textures that Properties names; the rest are counted.
const MAX_TEXTURES_LISTED: usize = 50;

/// What a scan is asked, all of it owned so a worker can read it.
#[derive(Debug, Clone)]
pub struct Ask {
    /// The `.openrage-proj`.
    pub project: PathBuf,
    pub folder: PathBuf,
    pub container: Container,
    /// The files the project holds: its documents, and every file open in a tab.
    pub members: Vec<PathBuf>,
    /// The plugins that read each plan, to validate it or to add files to it.
    pub extensions: Vec<Extension>,
}

/// What a scan found.
#[derive(Debug, Clone)]
pub struct Scan {
    pub project: PathBuf,
    pub rows: Vec<AssetRow>,
    pub plans: Vec<Planned>,
    /// The links of the project as the disk shows them: where each leads and who writes it.
    pub links: Vec<or_project::links::Link>,
    /// Why a kind of build cannot ship a content, which the matrix of the Builds window locks.
    pub locks: Vec<Lock>,
    /// The clothing content, each read and planned by the engine.
    pub packs: Vec<Pack>,
    /// What the plugins that extend a build said about its plans.
    pub said: Vec<Said>,
    /// How many peds each addon ped content holds, or why its model could not be read.
    pub peds: Vec<(String, Result<usize, String>)>,
    /// The weapons contents, each read and checked by the engine.
    pub weapons: Vec<super::weapons::WeaponsRead>,
    /// The maps contents, each read and checked by the engine.
    pub maps: Vec<super::maps::MapsRead>,
    /// The particles contents, each read and checked by the engine.
    pub particles: Vec<super::particles::ParticlesRead>,
    /// The walk stopped at its bounds, so a source folder further down may not be listed.
    pub truncated: bool,
}

/// The plans of one build as they stood when the scan read the disk: one, or one for each resource it splits into.
#[derive(Debug, Clone)]
pub struct Planned {
    pub output: String,
    pub resources: Vec<Resource>,
    /// What the files of each clothing content the build takes weigh by garment and slot, as this build's own options plan them.
    pub sizes: Vec<(String, or_wardrobe::sizes::Sizes)>,
}

/// The plan of one resource, and what it would change from the build that is there now.
#[derive(Debug, Clone)]
pub struct Resource {
    pub plan: Plan,
    /// None when no build is there, or the one that is recorded no items.
    pub since: Option<Diff>,
}

/// One row of the Assets table.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetRow {
    /// The path the asset is at, which a selection remembers.
    pub key: String,
    pub name: String,
    /// What it is, in words: `Texture dictionary`, `Textures source`.
    pub kind: String,
    /// The id of the content that holds it.
    pub content: String,
    pub kept: Kept,
    pub path: PathBuf,
    pub bytes: Option<u64>,
    pub seen: Seen,
    /// A source's stable id, which a rename or a move of its folder never changes.
    pub id: Option<String>,
    /// What else a source's `asset.json` says, as the Properties panel lists it.
    pub facts: Vec<(&'static str, String)>,
    /// Where the cook writes a source's output in the shadow folder.
    pub output: Option<PathBuf>,
    /// The textures a source holds: Properties lists their names and the quick look draws the first.
    pub textures: Vec<String>,
    /// The edition the asset is laid out for when its header or its manifest says: Legacy or Enhanced.
    pub target: Option<&'static str>,
    /// What a source's manifest says it holds, as the word that names its folder: `textures`, `drawable` or `drawables`.
    pub source_kind: Option<String>,
    /// A source whose manifest says its content is the game's file.
    pub from_game: bool,
    /// Where a source says its content came from, as one string two copies of the same file share: how, whether the game, the virtual path and the original's hash.
    pub origin: Option<String>,
}

/// How an asset's source is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// A file, copied as it is.
    File,
    /// A `<name>.<kind>.src` folder that is compiled.
    Source,
    /// A garment of a clothing content, which its pack holds and no file of its own states.
    Pack,
}

impl Kept {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => tr!("Loose file"),
            Self::Source => tr!("Source folder"),
            Self::Pack => tr!("In the clothing pack"),
        }
    }
}

/// What the disk showed of an asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    Present,
    Missing,
    /// A folder where the Files content holds files.
    Folder,
    /// A source with no output in the shadow folder yet.
    NotCooked,
    Cooked,
    /// A folder that is no source this build reads, and why.
    Unreadable(String),
}

/// Reads the project on a worker.
pub fn start(ask: Ask) -> Receiver<Scan> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(read(&ask));
    });
    rx
}

/// Reads the project: a row for each file it holds and each source folder in it, and the plan of each build.
pub fn read(ask: &Ask) -> Scan {
    let content = ask
        .container
        .modules
        .iter()
        .find(|m| m.kind == ModuleKind::Files && !m.is_link())
        .map_or("files", |m| m.id.as_str());
    let (folders, mut truncated) = find_sources(&ask.folder, &ask.members);
    // ⚠ The sources of a project whose Files content is linked are cooked into this project's builds, and listed in none of its rows.
    let (linked, linked_cut) = linked_sources(&ask.container, &ask.folder);
    truncated |= linked_cut;
    let mut cookable = describe_all(&folders);
    cookable.extend(describe_all(&linked));
    let shadow = shadow_root(&ask.project);

    let (packs, clothing) = read_packs(&ask.container, &ask.folder);

    let mut rows: Vec<AssetRow> = Vec::new();
    for member in &ask.members {
        if !folders.contains(member) {
            rows.push(file_row(member, content));
        }
    }
    let named = named_by_garments(&packs);
    for folder in &folders {
        // A source a garment names is that garment's row; one in the folder of a clothing content that no garment names is that content's, and the Files content does not hold it.
        if named.contains(folder) {
            continue;
        }
        let owner = or_project::content::content_owning(&ask.container, &ask.folder, folder);
        rows.push(source_row(
            folder,
            &shadow,
            owner.map_or(content, |m| m.id.as_str()),
        ));
    }
    rows.extend(garment_rows(&packs));
    rows.sort_by(|a, b| (a.name.to_lowercase(), &a.path).cmp(&(b.name.to_lowercase(), &b.path)));

    let held = held(&ask.members);
    let mut said = Vec::new();
    let plans = ask
        .container
        .outputs
        .iter()
        .filter_map(|output| {
            // ⚠ The packs were planned once for a FiveM resource with the default options; a build of another kind, or with options of its own, is planned for itself.
            let planned_for_it = match or_wardrobe::is_standard(output) {
                true => clothing.clone(),
                false => Vec::new(),
            };
            let made = or_content::plans_given(
                &ask.container,
                &output.id,
                &ask.folder,
                &held,
                &cookable,
                planned_for_it,
            )
            .ok()?;
            let root = build_root(&ask.project);
            // ⚠ The clothing is read once for the output, and only when a plugin asked for it.
            let pack = if ask.extensions.iter().any(|e| e.reads_pack) {
                pack_document(&ask.container, &ask.folder, &output.id)
            } else {
                Vec::new()
            };
            let resources = made
                .into_iter()
                .map(|plan| {
                    // ⚠ The plugins read the plan a build will make, so what they add and report is in the table before the build.
                    let extended = extend(&ask.extensions, plan, &pack);
                    said.extend(extended.said);
                    Resource {
                        since: since_last_build(&extended.plan, &root),
                        plan: extended.plan,
                    }
                })
                .collect();
            Some(Planned {
                output: output.id.clone(),
                resources,
                sizes: of_output(&ask.container, &ask.folder, output, &packs),
            })
        })
        .collect();
    Scan {
        project: ask.project.clone(),
        rows,
        plans,
        links: or_project::links::links(&ask.container, &ask.project),
        locks: locks_of(&ask.container, &ask.folder, &held, &cookable, &clothing),
        packs,
        said,
        peds: peds_read(&ask.container, &ask.folder),
        weapons: super::weapons::read_all(&ask.container, &ask.folder),
        maps: super::maps::read_all(&ask.container, &ask.folder),
        particles: super::particles::read_all(&ask.container, &ask.folder),
        truncated,
    }
}

/// How many peds each addon ped content of `container` holds, or why its model could not be read.
fn peds_read(container: &Container, folder: &Path) -> Vec<(String, Result<usize, String>)> {
    let model = |module: &or_project::Module| {
        let at = or_addonped::module_folder(folder, module).map_err(|e| e.to_string())?;
        let model = or_addonped::AddonPeds::read(&at).map_err(|e| e.to_string())?;
        Ok(model.peds.len())
    };
    (container.modules.iter())
        .filter(|m| m.kind == ModuleKind::AddonPed)
        .map(|m| (m.id.clone(), model(m)))
        .collect()
}

/// How many rows the Files content has: each member, and each source folder in the project's folder that no member names.
///
/// ⚠ The scan's own rule, which the start page's card counts by so it never says another number than the table.
pub fn files_held(container: &Container, folder: &Path, members: &[PathBuf]) -> usize {
    let (folders, _) = find_sources(folder, members);
    let own = (folders.iter())
        .filter(|f| or_project::content::content_owning(container, folder, f).is_none())
        .count();
    members
        .iter()
        .filter(|member| !folders.contains(member))
        .count()
        + own
}

/// The files a plan is made from, each as the disk shows it now.
pub fn held(members: &[PathBuf]) -> Vec<Held> {
    members
        .iter()
        .map(|path| Held {
            path: path.clone(),
            presence: presence(path),
        })
        .collect()
}

fn presence(path: &Path) -> Presence {
    match std::fs::metadata(path) {
        Err(_) => Presence::Missing,
        Ok(meta) if meta.is_dir() => Presence::Folder,
        Ok(meta) => Presence::File { bytes: meta.len() },
    }
}

fn file_row(path: &Path, content: &str) -> AssetRow {
    let found = presence(path);
    let seen = match found {
        Presence::Missing => Seen::Missing,
        Presence::Folder => Seen::Folder,
        Presence::File { .. } => Seen::Present,
    };
    let bytes = match found {
        Presence::File { bytes } => Some(bytes),
        _ => None,
    };
    // ⚠ The file's own header, not the install it came from: a file's target is its RSC7 version.
    let target = bytes
        .and_then(|_| read_file_generation(path))
        .map(|(generation, _)| generation.edition_label());
    AssetRow {
        key: path.display().to_string(),
        kind: kind_of(path),
        name: file_name(path),
        content: content.to_string(),
        kept: Kept::File,
        path: path.to_path_buf(),
        bytes,
        seen,
        id: None,
        facts: (target.map(|t| (tr!("Target"), t.to_string())).into_iter()).collect(),
        output: None,
        textures: Vec::new(),
        target,
        source_kind: None,
        from_game: false,
        origin: None,
    }
}

/// What a file is, in words: the kind the editor opens it as, else its extension.
fn kind_of(path: &Path) -> String {
    // ⚠ A member that is a `.src` folder and is not there is still said to be one, so Locate asks for a folder.
    if let Some((_, word)) = parse_folder_name(&file_name(path)) {
        return format!("{} source", capitalised(word));
    }
    if let Some(kind) = DocumentKind::from_path(path) {
        return kind.info().noun.to_string();
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(extension) => format!("{} file", extension.to_uppercase()),
        None => tr!("File").to_string(),
    }
}

fn source_row(folder: &Path, shadow: &Path, content: &str) -> AssetRow {
    let key = folder.display().to_string();
    let named = file_name(folder);
    let (stem, word) = parse_folder_name(&named).unwrap_or((&named, "source"));
    let unreadable = |why: String| AssetRow {
        key: key.clone(),
        name: stem.to_string(),
        kind: format!("{} source", capitalised(word)),
        content: content.to_string(),
        kept: Kept::Source,
        path: folder.to_path_buf(),
        bytes: None,
        seen: Seen::Unreadable(why),
        id: None,
        facts: Vec::new(),
        output: None,
        textures: Vec::new(),
        target: None,
        source_kind: None,
        from_game: false,
        origin: None,
    };
    let source = match Source::open(folder) {
        Ok(source) => source,
        Err(problem) => return unreadable(problem.message),
    };
    let manifest = &source.manifest;
    // ⚠ The cook's own naming: a drawable source compiles to a .ydr, a drawables source to a .ydd, a fragment to a .yft, a bounds source to a .ybn, a paths source to a .ynd, and a metadata source to the extension its manifest states.
    let is_model = models::fixed_file(&manifest.kind).is_some();
    let is_metadata = manifest.kind == metadata::KIND;
    let is_particles = manifest.kind == particles::KIND;
    let is_bounds = manifest.kind == bounds::KIND;
    let is_paths = manifest.kind == paths::KIND;
    let cooked = if is_model {
        models::output_name(manifest)
    } else if is_particles {
        particles::output_name(manifest)
    } else if is_bounds {
        bounds::output_name(manifest)
    } else if is_paths {
        paths::output_name(manifest)
    } else {
        metadata::cooked_name(manifest)
            .ok()
            .filter(|_| is_metadata)
            .unwrap_or_else(|| textures::output_name(manifest))
    };
    let output = shadow.join(&manifest.id).join(&cooked);
    let (bytes, listed) = textures_of(folder);
    let of_pictures = !(is_model || is_metadata || is_particles || is_bounds || is_paths);
    let bytes = of_pictures.then_some(bytes);
    let target = Generation::from_slug(&manifest.target).map(Generation::edition_label);
    // ⚠ No target and no platform among the facts: the layout a source compiles to is its manifest's and nothing a person chooses, and a build compiles it for the platforms the build states.
    let mut facts = vec![
        (tr!("Came from"), came_from(&manifest.origin)),
        (tr!("Compiles to"), cooked.clone()),
        (tr!("Stable id"), manifest.id.clone()),
        (tr!("Created with"), manifest.created_with.clone()),
    ];
    if of_pictures {
        if let Some(from) = compiles_from(folder, &listed) {
            facts.push((tr!("Compiles from"), from));
        }
    }
    AssetRow {
        key,
        name: manifest.name.clone(),
        kind: format!("{} source", capitalised(&manifest.kind)),
        content: content.to_string(),
        kept: Kept::Source,
        path: folder.to_path_buf(),
        bytes,
        seen: if output.is_file() {
            Seen::Cooked
        } else {
            Seen::NotCooked
        },
        id: Some(manifest.id.clone()),
        facts,
        output: Some(output),
        textures: listed,
        target,
        source_kind: Some(manifest.kind.clone()),
        from_game: manifest.origin.from_game,
        origin: Some(origin_key(&manifest.origin)),
    }
}

/// Where a source's content came from, in a sentence a person reads.
fn came_from(origin: &or_source::asset::Origin) -> String {
    match (origin.how, origin.from_game, origin.virtual_path.as_deref()) {
        (_, true, Some(path)) => tr!("The game: {path}", path = path),
        (_, true, None) => tr!("The game").to_string(),
        (or_source::asset::How::Authored, ..) => tr!("Made in OpenRage").to_string(),
        (_, false, Some(path)) => tr!("A file you imported: {path}", path = path),
        (_, false, None) => tr!("A file you imported").to_string(),
    }
}

/// What a source says of where its content came from, which two copies of one source say alike: how, whether it is the game's, its virtual path and the hash of the original.
fn origin_key(origin: &or_source::asset::Origin) -> String {
    format!(
        "{:?}|{}|{}|{}",
        origin.how,
        origin.from_game,
        origin.virtual_path.as_deref().unwrap_or_default(),
        origin.sha256.as_deref().unwrap_or_default()
    )
}

/// A row for each garment of each clothing content the scan could read, so a pack of garments is not a project that holds no files.
fn garment_rows(packs: &[Pack]) -> Vec<AssetRow> {
    let mut rows = Vec::new();
    for pack in packs {
        let Ok(read) = &pack.read else {
            continue;
        };
        for (index, garment) in read.wardrobe.garments.iter().enumerate() {
            rows.push(garment_row(pack, read, index, garment));
        }
    }
    rows
}

/// The paths the garments of `packs` name, each in its pack's folder.
fn named_by_garments(packs: &[Pack]) -> std::collections::HashSet<PathBuf> {
    let mut named = std::collections::HashSet::new();
    for pack in packs {
        let Ok(read) = &pack.read else {
            continue;
        };
        for name in or_wardrobe::facts::named_paths(&read.wardrobe) {
            named.insert(
                pack.folder
                    .join(name.replace('/', std::path::MAIN_SEPARATOR_STR)),
            );
        }
    }
    named
}

/// The key of the row of the garment `id` of the clothing content `content`.
pub fn garment_key(content: &str, id: &str) -> String {
    format!("garment:{content}:{id}")
}

fn garment_row(pack: &Pack, read: &PackRead, index: usize, garment: &Garment) -> AssetRow {
    let model = garment.model.as_deref().map(|m| pack.folder.join(m));
    let bytes = (model.as_ref())
        .and_then(|m| std::fs::metadata(m).ok())
        .map(|m| m.len());
    let said = |word: Option<String>| word.unwrap_or_else(|| "not said yet".to_string());
    let position = read.allocation.of(index).map(|p| {
        let pinned = if garment.position.is_some() {
            tr!(", pinned")
        } else {
            ""
        };
        format!("{:03}{pinned}", p.place)
    });
    let facts = vec![
        (tr!("Stable id"), garment.id.clone()),
        (tr!("Ped"), said(garment.ped.map(|p| format!("{p:?}")))),
        (
            tr!("Slot"),
            said(garment.slot.map(|s| s.name().to_string())),
        ),
        (
            tr!("Position"),
            position.unwrap_or_else(|| "not placed".to_string()),
        ),
        (tr!("Textures"), garment.variations.len().to_string()),
        (
            tr!("Cloth"),
            if garment.cloth.is_some() { "yes" } else { "no" }.to_string(),
        ),
    ];
    let textures = (garment.variations.iter())
        .filter_map(|v| v.texture.as_deref())
        .map(|t| t.rsplit('/').next().unwrap_or(t).to_string())
        .take(MAX_TEXTURES_LISTED)
        .collect();
    AssetRow {
        key: garment_key(&pack.module, &garment.id),
        name: or_wardrobe::rules::name_of(garment)
            .trim_matches('`')
            .to_string(),
        kind: tr!("Garment").to_string(),
        content: pack.module.clone(),
        kept: Kept::Pack,
        path: model.unwrap_or_else(|| pack.folder.clone()),
        bytes,
        seen: Seen::Present,
        id: Some(garment.id.clone()),
        facts,
        output: None,
        textures,
        target: None,
        source_kind: None,
        from_game: false,
        origin: None,
    }
}

/// Which file each texture of a textures source compiles from, and what it wins over: its DDS, the image it is made from, or the images whose channels it packs.
///
/// ⚠ `or_source::textures::inputs_of` says it, from the names and the rules and with no file read, so what the inspector shows is what the compile uses. A source with
/// thousands of textures lists the first few dozen and says how many more there are.
fn compiles_from(folder: &Path, files: &[String]) -> Option<String> {
    let inputs = Source::open(folder)
        .ok()
        .and_then(|source| textures::inputs_of(&source).ok());
    let Some(inputs) = inputs else {
        // A source whose rules cannot be read is listed by its files, which is all there is to say.
        return (!files.is_empty()).then(|| {
            files.join(
                "
",
            )
        });
    };
    let mut lines: Vec<String> = inputs
        .iter()
        .take(MAX_TEXTURES_LISTED)
        .map(textures::Input::sentence)
        .collect();
    if inputs.len() > MAX_TEXTURES_LISTED {
        lines.push(tr!(
            "… and {more} more",
            more = inputs.len() - MAX_TEXTURES_LISTED
        ));
    }
    (!lines.is_empty()).then(|| {
        lines.join(
            "
",
        )
    })
}

/// The bytes of the files in a textures source's `textures/` folder, and the names of its textures.
fn textures_of(folder: &Path) -> (u64, Vec<String>) {
    let mut bytes = 0;
    let mut names = Vec::new();
    let listing = std::fs::read_dir(folder.join(textures::DIR));
    for entry in listing.into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if !textures::is_texture_file(&file) {
            continue;
        }
        bytes += entry.metadata().map_or(0, |m| m.len());
        names.push(file);
    }
    names.sort();
    if names.len() > MAX_TEXTURES_LISTED {
        let more = names.len() - MAX_TEXTURES_LISTED;
        names.truncate(MAX_TEXTURES_LISTED);
        names.push(tr!("… and {more} more", more = more));
    }
    (bytes, names)
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn capitalised(word: &str) -> String {
    let mut letters = word.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_source(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("asset.json"), "{}").unwrap();
    }

    fn names(found: &[PathBuf]) -> Vec<String> {
        found.iter().map(|p| file_name(p)).collect()
    }

    #[test]
    fn the_inspector_says_which_file_a_texture_compiles_from_and_what_that_file_wins_over() {
        let dir = tempfile::tempdir().unwrap();
        let ytd = or_source::sample::texture_dictionary("art", 90);
        let folder = or_source::game_import::decompile(
            &ytd,
            "art.ytd",
            dir.path(),
            &or_source::game_import::Facts::default(),
        )
        .unwrap();
        let (_, listed) = textures_of(&folder);

        let alone = compiles_from(&folder, &listed).expect("the DDS is listed");
        assert!(
            alone.contains("art.dds (DDS)") && alone.contains("copied as it is"),
            "{alone}"
        );

        // An image of the texture's name wins over its DDS, and the inspector says so and names what it wins over.
        std::fs::write(folder.join("textures/art.png"), b"image").unwrap();
        let (_, listed) = textures_of(&folder);
        let both = compiles_from(&folder, &listed).expect("the winner is listed");
        assert!(
            both.contains("art.png (image) wins") && both.contains("art.dds beside it"),
            "{both}"
        );
    }

    #[test]
    fn a_walk_finds_sources_below_the_project_and_reads_neither_what_a_build_wrote_nor_a_source_inside_a_source(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        a_source(&root.join("a/b/x.textures.src"));
        a_source(&root.join("p.textures.src"));
        a_source(&root.join("p.textures.src/inner/q.textures.src"));
        a_source(&root.join("build/y.textures.src"));
        a_source(&root.join("vfs/z.textures.src"));
        a_source(&root.join(".hidden/w.textures.src"));

        let (found, truncated) = find_sources(root, &[]);

        assert_eq!(names(&found), ["x.textures.src", "p.textures.src"]);
        assert!(!truncated);
    }

    #[test]
    fn a_member_folder_that_is_gone_is_a_source_by_its_name_and_a_file_is_what_its_kind_says() {
        assert_eq!(kind_of(Path::new("a/logo.textures.src")), "Textures source");
        assert_eq!(kind_of(Path::new("lamp.drawable.src")), "Drawable source");
        assert_eq!(kind_of(Path::new("a/client.lua")), "Text Document");
        assert_eq!(kind_of(Path::new("a/x.src")), "SRC file");
    }

    #[test]
    fn a_walk_that_stops_at_its_depth_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut deep = dir.path().to_path_buf();
        for level in 0..=or_project::SEARCH_DEPTH + 1 {
            deep.push(format!("l{level}"));
        }
        a_source(&deep.join("far.textures.src"));

        let (found, truncated) = find_sources(dir.path(), &[]);

        assert!(found.is_empty(), "{found:?}");
        assert!(
            truncated,
            "a source too deep to find is not silently missing"
        );
    }

    #[test]
    fn a_member_folder_that_is_a_source_is_one_row_and_a_source_with_no_header_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("proj");
        let broken = folder.join("logo.textures.src");
        a_source(&broken);
        let ask = Ask {
            project: folder.join("p.openrage-proj"),
            folder: folder.clone(),
            container: Container::default(),
            members: vec![broken.clone()],
            extensions: Vec::new(),
        };

        let scan = read(&ask);

        assert_eq!(scan.rows.len(), 1, "{:?}", scan.rows);
        assert_eq!(scan.rows[0].kept, Kept::Source);
        assert_eq!(scan.rows[0].name, "logo");
        assert!(
            matches!(scan.rows[0].seen, Seen::Unreadable(_)),
            "{:?}",
            scan.rows[0].seen
        );
        assert!(scan.plans.is_empty(), "no build is declared");
    }

    #[test]
    fn a_source_shows_the_output_its_cook_makes_whatever_its_kind() {
        use or_source::asset::{folder_name, AssetManifest, How, Origin, MANIFEST};
        use or_source::{bounds, fragment, metadata, paths};

        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("proj");
        for (kind, name) in [
            (models::DRAWABLE, "lamp"),
            (models::DRAWABLES, "props"),
            (fragment::KIND, "van"),
            (metadata::KIND, "cs1_02"),
            (bounds::KIND, "cs1_02_0"),
            (paths::KIND, "nodes7"),
        ] {
            let source = folder.join("sources").join(folder_name(name, kind));
            std::fs::create_dir_all(&source).unwrap();
            let origin = Origin {
                how: How::Authored,
                from_game: false,
                virtual_path: None,
                game_build: None,
                sha256: None,
                cooked_sha256: None,
                extra: Default::default(),
            };
            let mut manifest = AssetManifest::new(kind, name, "gen8", origin).unwrap();
            if kind == metadata::KIND {
                manifest.extra.insert(
                    "compile".to_string(),
                    serde_json::json!({ "extension": "ymap", "base": "" }),
                );
            }
            std::fs::write(source.join(MANIFEST), manifest.to_text()).unwrap();
            let fixed = match kind {
                bounds::KIND => bounds::RULES,
                paths::KIND => paths::RULES,
                kind => models::fixed_file(kind).unwrap_or(metadata::XML),
            };
            std::fs::create_dir_all(source.join(fixed).parent().unwrap()).unwrap();
            std::fs::write(source.join(fixed), "{}").unwrap();
        }
        let ask = Ask {
            project: folder.join("p.openrage-proj"),
            folder: folder.clone(),
            container: Container::default(),
            members: Vec::new(),
            extensions: Vec::new(),
        };

        let scan = read(&ask);

        let named = |name: &str| scan.rows.iter().find(|row| row.name == name).expect(name);
        let (lamp, props) = (named("lamp"), named("props"));
        assert_eq!(lamp.kind, "Drawable source");
        assert_eq!(props.kind, "Drawables source");
        let extension = |row: &AssetRow| {
            let output = row.output.as_ref().expect("an output");
            output
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_string)
        };
        assert_eq!(extension(lamp).as_deref(), Some("ydr"));
        assert_eq!(extension(props).as_deref(), Some("ydd"));
        let (van, map) = (named("van"), named("cs1_02"));
        assert_eq!(van.kind, "Fragment source");
        assert_eq!(map.kind, "Metadata source");
        assert_eq!(extension(van).as_deref(), Some("yft"));
        assert_eq!(extension(map).as_deref(), Some("ymap"));
        let (collision, roads) = (named("cs1_02_0"), named("nodes7"));
        assert_eq!(collision.kind, "Bounds source");
        assert_eq!(roads.kind, "Paths source");
        assert_eq!(extension(collision).as_deref(), Some("ybn"));
        assert_eq!(extension(roads).as_deref(), Some("ynd"));
        assert_eq!((collision.bytes, roads.bytes), (None, None));
    }
}

#[cfg(test)]
mod a_plugin_extends_what_a_project_builds;
