//! The plan of a build: what an output will write, as data, before anything is written.
//!
//! A pure function from the container and what its modules contribute; the plan itself touches no
//! disk. The Files module contributes the files the caller has looked at and reports as
//! [`Source`]s, and any other kind of content hands over a [`Contribution`] of its own. The one
//! exception is a Files content that is a link, which reads the project file it leads to
//! ([`crate::links::linked_files`]) when the contributions are made, and never in the plan.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use or_io::gta5::retarget::CONVERTED;

use crate::container::{Container, ModuleKind, Output, OutputKind, Target};
use crate::contribution::{
    ArchiveContents, Claim, Contribution, Item, ManifestEntry, SourceFolder,
};
use crate::paths::{folded, is_under, is_within};
use crate::{dlc, merge, oiv};

/// The one document a FiveM resource needs that a loose file cannot be.
pub const MANIFEST: &str = "fxmanifest.lua";

/// A file the project holds, and what the caller saw of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub path: PathBuf,
    pub presence: Presence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Missing,
    Folder,
    File { bytes: u64 },
}

/// What a build of one output will write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The output's id.
    pub output: String,
    /// The folder the build writes, which is the output's name.
    pub resource: String,
    /// The project's folder, which a build never replaces and never deletes anything in.
    pub project_folder: PathBuf,
    /// Every file, ordered by path.
    pub files: Vec<PlannedFile>,
    /// The content that wrote each file that a content wrote, by the file's path; the manifest the build writes is in none.
    pub made_by: BTreeMap<String, String>,
    pub totals: Totals,
    pub limits: Limits,
    /// The figures that contents contributed, summed, against the ceilings the output declares.
    pub budgets: Vec<Budget>,
    /// The stable-id items of every content the output takes, ordered by id.
    pub items: Vec<Item>,
    /// The names the game resolves once that the plan checked, and the ones two contents hold.
    pub names: NameCheck,
    /// Each name the game resolves once that a content this plan takes claims, in the order the contents state them.
    pub claims: Vec<Claim>,
    /// What the output writes, which decides how a build writes it and reads it back.
    pub kind: OutputKind,
    /// The layout the output ships its drawables and texture dictionaries in; as authored when none.
    pub target: Option<Target>,
    /// Whether a cooked source of the game's own content ships when the cook leaves it as the game has it.
    pub ship_unchanged: bool,
    /// The files a singleplayer DLC registers in its `content.xml`, and the type each is registered as.
    pub registrations: Vec<Registration>,
    /// The path a pack is mounted from, which `dlclist.xml` needs a line for; only a singleplayer DLC has one.
    pub dlclist_entry: Option<String>,
    /// Why this plan cannot be built, one sentence each; a plan with any is refused.
    pub problems: Vec<String>,
    /// What a person is told that does not stop the build, one sentence each: a file it names by its id and does not ship.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// Inside the resource, with `/` on every host.
    pub path: String,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A file of the project, copied exactly.
    Copy { from: PathBuf, bytes: u64 },
    /// A document the build writes.
    Document(String),
    /// A file the build writes whose bytes are not text, such as a binary metadata file.
    Generated(Vec<u8>),
    /// A file of the project moved to the plan's target, or copied when it is there already; a build decides.
    /// `bytes` is the source's size, since the moved file's is not known before it is made.
    Retarget { from: PathBuf, bytes: u64 },
    /// A `.src` folder of the project that the build cooks into the file it ships, then moves to the plan's target like a retarget.
    ///
    /// `bytes` is the payload the file is made from, since the cooked file's size is not known before it is made.
    /// `game` holds the SHA-256 of each file that is the game's own, empty for a source that is not.
    Cook {
        source: PathBuf,
        kind: String,
        name: String,
        bytes: u64,
        game: Vec<String>,
    },
}

/// A file a pack registers, by its path in the pack, and the type the game is told it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub path: String,
    pub file_type: String,
    /// What the pack states the path holds; empty where it states nothing.
    pub contents: String,
    /// Whether the path is a name the game asks by and no file of the pack: the `.ityp` of a `.ytyp` inside a nested archive.
    pub asked: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub files: u64,
    pub bytes: u64,
}

/// The ceilings the output declares, none when it declares none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    pub files: Option<u64>,
    pub bytes: Option<u64>,
}

/// One named figure, summed over the contents the output takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    pub name: String,
    pub used: u64,
    /// The output's `limits` entry for the name, none when it declares none.
    pub ceiling: Option<u64>,
}

/// The names of one kind that the game resolves once, counted, and the ones two contents hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameCheck {
    /// How many names of each kind the contents of the build claim.
    pub held: BTreeMap<String, usize>,
    /// One sentence for each name two claims hold, which is a problem of the plan too.
    pub clashes: Vec<String>,
}

impl NameCheck {
    /// The line the verification shows: the good news when no name is held twice, and how many are when some are.
    pub fn line(&self) -> String {
        let total: usize = self.held.values().sum();
        if !self.clashes.is_empty() {
            return format!(
                "{} name(s) the game resolves once are held twice",
                self.clashes.len()
            );
        }
        if total == 0 {
            return "No content holds a name the game resolves once".to_string();
        }
        let kinds: Vec<String> = (self.held.iter())
            .map(|(kind, n)| format!("{n} {kind}"))
            .collect();
        format!(
            "{total} name(s) the game resolves once, and no two contents hold one ({})",
            kinds.join(", ")
        )
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the project declares no build `{0}`")]
pub struct NoSuchOutput(pub String);

impl Plan {
    pub fn is_buildable(&self) -> bool {
        self.problems.is_empty()
    }

    /// Whether a total or a budget is over the ceiling the output declares for it, which a split can mend.
    pub fn over_a_limit(&self) -> bool {
        let over = |used: u64, ceiling: Option<u64>| ceiling.is_some_and(|max| used > max);
        over(self.totals.files, self.limits.files)
            || over(self.totals.bytes, self.limits.bytes)
            || self.budgets.iter().any(|b| over(b.used, b.ceiling))
    }
}

impl PlannedFile {
    pub fn bytes(&self) -> u64 {
        match &self.origin {
            Origin::Copy { bytes, .. } => *bytes,
            Origin::Document(text) => text.len() as u64,
            Origin::Retarget { bytes, .. } | Origin::Cook { bytes, .. } => *bytes,
            Origin::Generated(bytes) => bytes.len() as u64,
        }
    }
}

/// The plan of `output`, for the project in `folder` holding `sources`.
///
/// A Files module holds the project's documents, which land at the path they have inside the
/// project folder; when an output takes several, the first holds them and the others hold none.
/// The one document a build writes is `fxmanifest.lua`, unless the project holds its own; it lists
/// every file but the streamed ones, which FiveM finds in `stream/` by itself.
pub fn plan(
    container: &Container,
    output: &str,
    folder: &Path,
    sources: &[Source],
) -> Result<Plan, NoSuchOutput> {
    let contributions = files_contributions(container, output, folder, sources)?;
    plan_with(container, output, folder, &contributions)
}

/// What the Files modules of the project contribute to `output`.
///
/// A kind of content that arrives later adds its own contributions to these and plans them with [`plan_with`] or [`crate::plans_with`].
pub fn files_contributions(
    container: &Container,
    output: &str,
    folder: &Path,
    sources: &[Source],
) -> Result<Vec<Contribution>, NoSuchOutput> {
    files_contributions_with(container, output, folder, sources, &[])
}

/// [`files_contributions`] with the `.src` folders of the project, which the first Files module holds and a build cooks.
///
/// A `.src` folder among `sources`, and a file inside one, belong to the source and are not files
/// the Files content holds: a build cooks the folder and never ships it. A Files content that is a
/// link holds the files of the project it leads to ([`crate::links::linked_files`]), so `folders`
/// also holds the source folders of those projects ([`crate::links::linked_sources`]).
pub fn files_contributions_with(
    container: &Container,
    output: &str,
    folder: &Path,
    sources: &[Source],
    folders: &[SourceFolder],
) -> Result<Vec<Contribution>, NoSuchOutput> {
    let declared = container
        .output(output)
        .ok_or_else(|| NoSuchOutput(output.to_string()))?;
    let linked: Vec<PathBuf> = (container.modules.iter())
        .filter(|m| m.kind == ModuleKind::Files && m.is_link())
        .filter_map(|m| crate::links::folder_of(m, folder).ok())
        .collect();
    let (theirs, ours): (Vec<&SourceFolder>, Vec<&SourceFolder>) = folders
        .iter()
        .partition(|f| linked.iter().any(|at| is_within(&folded(&f.folder), at)));
    let ours: Vec<SourceFolder> = ours.into_iter().cloned().collect();
    let theirs: Vec<SourceFolder> = theirs.into_iter().cloned().collect();
    let files: Vec<Source> = sources
        .iter()
        .filter(|s| {
            !ours
                .iter()
                .any(|f| is_within(&folded(&s.path), &folded(&f.folder)))
        })
        .cloned()
        .collect();
    // ⚠ The project's own files are held by its first Files content that is not a link.
    let holder = declared.take.iter().find(|id| {
        container
            .module(id)
            .is_some_and(|m| m.kind == ModuleKind::Files && !m.is_link())
    });
    Ok(container
        .modules
        .iter()
        .filter(|m| m.kind == ModuleKind::Files)
        .map(|m| {
            if m.is_link() {
                if declared.take.contains(&m.id) {
                    crate::links::linked_files(m, folder, &theirs)
                } else {
                    Contribution::of(&m.id)
                }
            } else if holder == Some(&m.id) {
                Contribution::files(&m.id, folder, &files)
                    .cooked(folder, &ours)
                    .with_file_types(m.extra.get("file_types"))
            } else {
                Contribution::of(&m.id)
            }
        })
        .collect())
}

/// Why a build of `kind` cannot ship what `contribution` hands over, none when it can.
///
/// The cell of a content and a build that a screen locks: a singleplayer DLC has no manifest and runs no script,
/// so a content that adds a manifest line, or files whose type the game is not told (a script among them), cannot ship in one.
/// An `.oiv` installs a content laid out as a pack inside such a pack, so the same holds of that content there.
pub fn cannot_ship(contribution: &Contribution, kind: &OutputKind) -> Option<String> {
    match kind {
        OutputKind::SingleplayerDlc => dlc::refuses(contribution),
        OutputKind::OivPackage if contribution.pack => dlc::refuses(contribution),
        _ => None,
    }
}

/// The files `contribution` hands over that a singleplayer DLC is told no type for, by path: what the Builds window lists under File types.
pub fn files_without_a_type(contribution: &Contribution) -> Vec<String> {
    dlc::untyped(contribution)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// The plan of `output` from what the modules contribute, one [`Contribution`] for each module the caller can build.
///
/// A module the output takes and nothing contributed for is refused, whatever its kind. The
/// contributions of modules it does not take still count for the names the game resolves once,
/// because those are one namespace for the whole project.
pub fn plan_with(
    container: &Container,
    output: &str,
    folder: &Path,
    contributions: &[Contribution],
) -> Result<Plan, NoSuchOutput> {
    let declared = container
        .output(output)
        .ok_or_else(|| NoSuchOutput(output.to_string()))?;
    Ok(assemble(
        container,
        declared,
        &declared.take,
        &declared.name,
        folder,
        contributions,
    ))
}

/// The plan that writes `resource` from the contents `take`, under the ceilings and options of `declared`.
pub(crate) fn assemble(
    container: &Container,
    declared: &Output,
    take: &[String],
    resource: &str,
    folder: &Path,
    contributions: &[Contribution],
) -> Plan {
    let limits = Limits {
        files: declared.max_files,
        bytes: declared.max_bytes,
    };
    let mut plan = Plan {
        output: declared.id.clone(),
        resource: resource.to_string(),
        project_folder: folder.to_path_buf(),
        files: Vec::new(),
        made_by: BTreeMap::new(),
        totals: Totals::default(),
        limits,
        budgets: Vec::new(),
        items: Vec::new(),
        names: NameCheck::default(),
        claims: Vec::new(),
        kind: declared.kind.clone(),
        target: declared.target,
        ship_unchanged: declared.ship_unchanged,
        registrations: Vec::new(),
        dlclist_entry: None,
        problems: Vec::new(),
        notes: Vec::new(),
    };
    if let OutputKind::Unknown(kind) = &declared.kind {
        plan.problems.push(format!(
            "build `{}` is of kind `{kind}`, which this version of OpenRage cannot make",
            declared.id
        ));
        return plan;
    }

    let mut taken: Vec<&Contribution> = Vec::new();
    for (index, id) in take.iter().enumerate() {
        if take[..index].contains(id) {
            continue;
        }
        let Some(module) = container.module(id) else {
            plan.problems.push(format!(
                "build `{}` takes `{id}`, which is not content of the project",
                declared.id
            ));
            continue;
        };
        match contributions.iter().find(|c| c.module == *id) {
            Some(contribution) => taken.push(contribution),
            None => plan.problems.push(match &module.kind {
                ModuleKind::Unknown(word) => {
                    format!("content `{id}` is of type `{word}`, which this build cannot build")
                }
                _ => format!(
                    "nothing contributed for content `{id}`, so what it holds would be left out"
                ),
            }),
        }
    }

    // Each file with the content that wrote it, and the document the build writes with none.
    let mut owned: Vec<(&str, PlannedFile)> = Vec::new();
    for contribution in &taken {
        let module = contribution.module.as_str();
        owned.extend(contribution.files.iter().map(|f| (module, f.clone())));
        plan.problems.extend(contribution.problems.iter().cloned());
        // ⚠ A content is asked for this build, so what locks the kind it laid its files out for stops this plan.
        plan.problems.extend(contribution.locks.iter().cloned());
        plan.notes.extend(contribution.notes.iter().cloned());
    }
    if declared.target.is_some() {
        for (_, file) in &mut owned {
            retarget(file);
        }
    }
    let entries: Vec<(&str, &ManifestEntry)> = taken
        .iter()
        .flat_map(|c| c.manifest.iter().map(move |line| (c.module.as_str(), line)))
        .collect();
    let packs: BTreeSet<&str> = (taken.iter())
        .filter(|c| c.pack)
        .map(|c| c.module.as_str())
        .collect();
    let archives: Vec<(&str, &ArchiveContents)> = taken
        .iter()
        .flat_map(|c| c.archives.iter().map(move |held| (c.module.as_str(), held)))
        .collect();
    match &declared.kind {
        OutputKind::FivemResource => manifest_of(&mut owned, &entries, declared, &mut plan),
        OutputKind::SingleplayerDlc => dlc::finish(&mut owned, &entries, &archives, &mut plan),
        OutputKind::OivPackage => oiv::finish(
            &mut owned,
            (&entries, &archives, &packs),
            declared,
            &mut plan,
        ),
        OutputKind::Unknown(_) => unreachable!("an unknown kind returned above"),
    }
    owned.sort_by(|a, b| a.1.path.cmp(&b.1.path));

    // ⚠ Two spellings that differ only in case are one path on Windows and on a Mac.
    let mut seen: HashMap<String, (&PlannedFile, &str)> = HashMap::with_capacity(owned.len());
    for (module, file) in &owned {
        let folded = file.path.to_lowercase();
        match seen.get(&folded) {
            Some((first, by)) => {
                let who = if by == module {
                    String::new()
                } else {
                    format!(" (content `{by}` and content `{module}`)")
                };
                plan.problems.push(format!(
                    "`{}` and `{}` are one path in the resource{who}{}{}",
                    first.path,
                    file.path,
                    cooked_by(first),
                    cooked_by(file)
                ));
            }
            None => {
                seen.insert(folded, (file, module));
            }
        }
    }

    let in_the_project: Vec<&Contribution> = contributions
        .iter()
        .filter(|c| container.module(&c.module).is_some())
        .collect();
    let taken_ids: BTreeSet<&str> = taken.iter().map(|c| c.module.as_str()).collect();
    plan.names = merge::claims(&in_the_project, &taken_ids, &mut plan.problems);
    plan.claims = (taken.iter())
        .flat_map(|c| c.claims.iter().cloned())
        .collect();
    plan.budgets = merge::budgets(&taken, declared, &mut plan.problems);
    plan.items = merge::items(&taken, &mut plan.problems);
    // ⚠ A pack is an archive, and so is each archive in it, and the game corrupts one that holds too many entries.
    let whole = match declared.kind {
        OutputKind::SingleplayerDlc => Some(true),
        OutputKind::OivPackage if !packs.is_empty() => Some(false),
        _ => None,
    };
    if let Some(whole) = whole {
        let paths: Vec<&str> = owned.iter().map(|(_, f)| f.path.as_str()).collect();
        dlc::count_entries(
            &paths,
            whole,
            declared,
            &mut plan.budgets,
            &mut plan.problems,
        );
    }

    plan.made_by = (owned.iter())
        .filter(|(module, _)| !module.is_empty())
        .map(|(module, file)| (file.path.clone(), (*module).to_string()))
        .collect();
    plan.files = owned.into_iter().map(|(_, file)| file).collect();
    plan.totals = Totals {
        files: plan.files.len() as u64,
        bytes: plan.files.iter().map(PlannedFile::bytes).sum(),
    };
    let ceilings = ceiling_problems(&plan);
    plan.problems.extend(ceilings);
    plan.notes.extend(crate::left_out::note(&plan));
    plan
}

/// ` (the source folder cooks it)` for a file a build cooks, so a clash says which folder made the file.
fn cooked_by(file: &PlannedFile) -> String {
    match &file.origin {
        Origin::Cook { source, .. } => format!(" (`{}` cooks `{}`)", source.display(), file.path),
        _ => String::new(),
    }
}

/// One sentence for each of the plan's own totals that is over the ceiling its output declares.
pub(crate) fn ceiling_problems(plan: &Plan) -> Vec<String> {
    let ceilings = [
        ("files", plan.totals.files, plan.limits.files),
        ("bytes", plan.totals.bytes, plan.limits.bytes),
    ];
    ceilings
        .into_iter()
        .filter_map(|(what, total, ceiling)| {
            let max = ceiling.filter(|max| total > *max)?;
            Some(format!(
                "the build writes {total} {what}, over the {max} that build `{}` allows",
                plan.output
            ))
        })
        .collect()
}

/// A copy of a file that has a layout per target becomes a retarget, which a build decides file by file.
pub(crate) fn retarget(file: &mut PlannedFile) {
    let ext = file.path.rsplit('.').next().unwrap_or_default();
    if !CONVERTED.contains(&ext.to_ascii_lowercase().as_str()) {
        return;
    }
    if let Origin::Copy { from, bytes } = &file.origin {
        file.origin = Origin::Retarget {
            from: from.clone(),
            bytes: *bytes,
        };
    }
}

/// The words a build states about itself, which the manifest of a FiveM resource carries in this order.
const WORDS: [&str; 3] = ["author", "description", "version"];

/// The manifest of a FiveM resource, unless the resource holds its own.
fn manifest_of<'a>(
    owned: &mut Vec<(&'a str, PlannedFile)>,
    entries: &[(&'a str, &ManifestEntry)],
    declared: &Output,
    plan: &mut Plan,
) {
    let stated = merge::stated(declared, &WORDS, &mut plan.problems);
    if owned
        .iter()
        .any(|(_, f)| f.path.eq_ignore_ascii_case(MANIFEST))
    {
        if !stated.is_empty() {
            let keys: Vec<&str> = stated.iter().map(|(key, _)| *key).collect();
            plan.problems.push(format!(
                "build `{}` states {}, but the resource holds its own {MANIFEST}, which a build does not edit",
                declared.id,
                keys.join(", ")
            ));
        }
        let mut adders: Vec<&str> = entries.iter().map(|(module, _)| *module).collect();
        adders.dedup();
        for module in adders {
            plan.problems.push(format!(
                "content `{module}` adds manifest lines, but the resource holds its own {MANIFEST}, which a build does not edit"
            ));
        }
    } else {
        let mut listed: Vec<&str> = owned
            .iter()
            .map(|(_, f)| f.path.as_str())
            .filter(|path| !is_under(path, "stream"))
            // ⚠ A server reads the entitlement of an asset escrow itself: listed, it would be sent to every client.
            .filter(|path| *path != crate::place::ESCROW_ENTITLEMENT)
            .collect();
        listed.sort_unstable();
        let text = merge::manifest(&listed, &stated, entries, &mut plan.problems);
        owned.push((
            "",
            PlannedFile {
                path: MANIFEST.to_string(),
                origin: Origin::Document(text),
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{Module, Output};

    const FOLDER: &str = "/projects/pack";

    fn container() -> Container {
        Container {
            modules: vec![Module::files("files")],
            outputs: vec![Output::fivem_resource("fivem", "pack", &["files"])],
        }
    }

    /// A file at `rel` inside the project folder, which is nowhere on the disk.
    fn file(rel: &str, bytes: u64) -> Source {
        Source {
            path: Path::new(FOLDER).join(rel),
            presence: Presence::File { bytes },
        }
    }

    fn planned(container: &Container, sources: &[Source]) -> Plan {
        plan(container, "fivem", Path::new(FOLDER), sources).expect("the output is declared")
    }

    fn paths(plan: &Plan) -> Vec<&str> {
        plan.files.iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn a_plan_lists_every_copy_and_the_manifest_in_path_order() {
        // ⚠ None of these exists: the plan is a function of what it is told.
        let sources = [
            file("stream/b.ydr", 30),
            file("data/vehicles.meta", 20),
            file("client.lua", 10),
            file("stream/a.ytd", 5),
        ];
        let plan = planned(&container(), &sources);

        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert_eq!(plan.output, "fivem");
        assert_eq!(plan.resource, "pack");
        assert_eq!(
            paths(&plan),
            [
                "client.lua",
                "data/vehicles.meta",
                "fxmanifest.lua",
                "stream/a.ytd",
                "stream/b.ydr"
            ]
        );
        let copies = plan
            .files
            .iter()
            .filter(|f| matches!(f.origin, Origin::Copy { .. }))
            .count();
        assert_eq!(copies, 4);
        let Origin::Copy { from, bytes } = &plan.files[0].origin else {
            panic!("client.lua is a copy")
        };
        assert_eq!(
            (from.as_path(), *bytes),
            (Path::new(FOLDER).join("client.lua").as_path(), 10)
        );

        let manifest = plan
            .files
            .iter()
            .find(|f| f.path == MANIFEST)
            .expect("a manifest");
        let Origin::Document(text) = &manifest.origin else {
            panic!("the manifest is a document")
        };
        assert!(
            text.starts_with("fx_version 'cerulean'\ngame 'gta5'\n"),
            "{text}"
        );
        assert!(
            text.contains("'client.lua',") && text.contains("'data/vehicles.meta',"),
            "{text}"
        );
        assert!(
            !text.contains("stream/"),
            "FiveM streams that folder by itself: {text}"
        );
        assert!(text.ends_with("}\n"), "{text:?}");
        assert_eq!(plan.totals.files, 5);
        assert_eq!(plan.totals.bytes, 10 + 20 + 5 + 30 + text.len() as u64);
    }

    #[test]
    fn a_resource_of_streamed_files_alone_lists_none() {
        let plan = planned(&container(), &[file("stream/a.ytd", 5)]);
        let Origin::Document(text) = &plan.files[0].origin else {
            panic!("fxmanifest.lua sorts first")
        };
        assert_eq!(text, "fx_version 'cerulean'\ngame 'gta5'\n");
    }

    #[test]
    fn a_project_that_holds_its_own_manifest_keeps_it() {
        let plan = planned(
            &container(),
            &[file("FxManifest.lua", 90), file("a.lua", 1)],
        );
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert_eq!(paths(&plan), ["FxManifest.lua", "a.lua"]);
        assert!(plan
            .files
            .iter()
            .all(|f| matches!(f.origin, Origin::Copy { .. })));
    }

    #[test]
    fn a_name_with_a_quote_is_written_so_the_manifest_still_reads() {
        let plan = planned(&container(), &[file("it's.meta", 1)]);
        let Origin::Document(text) = &plan.files[0].origin else {
            panic!("fxmanifest.lua sorts first")
        };
        assert!(text.contains("'it\\'s.meta',"), "{text}");
    }

    #[test]
    fn a_plan_refuses_what_it_cannot_place() {
        let folder = Path::new(FOLDER);
        let cases: Vec<(&str, Source, &str)> = vec![
            (
                "a missing file",
                Source {
                    path: folder.join("gone.ydr"),
                    presence: Presence::Missing,
                },
                "missing",
            ),
            (
                "a folder",
                Source {
                    path: folder.join("models"),
                    presence: Presence::Folder,
                },
                "folder",
            ),
            (
                "a file outside the project",
                Source {
                    path: PathBuf::from("/elsewhere/a.ydr"),
                    presence: Presence::File { bytes: 1 },
                },
                "outside",
            ),
            ("a file under build/", file("build/pack/a.ydr", 1), "build"),
            ("a file under BUILD/", file("BUILD/a.ydr", 1), "build"),
            ("a name Windows refuses", file("what?.lua", 1), "refuses"),
            ("a trailing dot", file("stream/a.lua.", 1), "refuses"),
        ];
        for (what, source, says) in cases {
            let plan = planned(&container(), &[source]);
            assert!(!plan.is_buildable(), "{what} was planned");
            assert!(
                plan.problems
                    .iter()
                    .any(|p| p.to_lowercase().contains(says)),
                "{what}: {:?}",
                plan.problems
            );
        }
    }

    #[test]
    fn two_files_that_one_path_names_are_refused() {
        let plan = planned(&container(), &[file("a.lua", 1), file("A.LUA", 1)]);
        assert!(
            plan.problems
                .iter()
                .any(|p| p.contains("a.lua") && p.contains("A.LUA")),
            "{:?}",
            plan.problems
        );
        let one = planned(&container(), &[file("a.lua", 1), file("a.lua", 1)]);
        assert!(
            !one.is_buildable(),
            "the same file twice would be written twice"
        );
    }

    #[test]
    fn an_output_takes_only_modules_that_exist_and_that_this_build_can_build() {
        let mut container = container();
        container.modules.push(Module {
            id: "uniforms".into(),
            kind: ModuleKind::Unknown("widgets".into()),
            extra: Default::default(),
        });
        container.outputs[0].take = vec!["files".into(), "uniforms".into(), "nowhere".into()];
        let plan = planned(&container, &[file("a.lua", 1)]);
        assert_eq!(plan.problems.len(), 2, "{:?}", plan.problems);
        assert!(plan
            .problems
            .iter()
            .any(|p| p.contains("uniforms") && p.contains("widgets")));
        assert!(plan.problems.iter().any(|p| p.contains("nowhere")));
    }

    #[test]
    fn an_output_that_takes_nothing_writes_only_its_manifest() {
        let mut container = container();
        container.outputs[0].take.clear();
        let plan = planned(&container, &[file("a.lua", 1)]);
        assert_eq!(paths(&plan), ["fxmanifest.lua"]);
    }

    #[test]
    fn an_output_this_build_does_not_know_is_refused() {
        let mut container = container();
        container.outputs[0].kind = crate::container::OutputKind::Unknown("installer".into());
        let plan = planned(&container, &[file("a.lua", 1)]);
        assert!(
            plan.problems.iter().any(|p| p.contains("installer")),
            "{:?}",
            plan.problems
        );
    }

    #[test]
    fn the_ceilings_an_output_declares_are_checked_against_the_totals() {
        let mut container = container();
        container.outputs[0].max_files = Some(2);
        container.outputs[0].max_bytes = Some(100);
        let plan = planned(&container, &[file("a.lua", 60), file("b.lua", 60)]);
        assert_eq!(
            plan.limits,
            Limits {
                files: Some(2),
                bytes: Some(100)
            }
        );
        assert_eq!(plan.problems.len(), 2, "{:?}", plan.problems);

        let within = planned(&container, &[file("a.lua", 1)]);
        assert!(within.is_buildable(), "{:?}", within.problems);
    }

    #[test]
    fn an_output_that_is_not_declared_is_an_error() {
        let error = plan(&container(), "nope", Path::new(FOLDER), &[]).unwrap_err();
        assert!(error.to_string().contains("nope"));
    }
}
