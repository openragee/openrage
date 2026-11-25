//! The executor: cook the sources, write a plan to a fresh folder, reload what was written, swap it in and keep the last build.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::gta5::retarget::retarget_file;
use or_io::gta5::shaders::shader_defs::ParamRemap;

use crate::container::{is_name, OutputKind};
use crate::content::facts::digest;
use crate::diff::{self, Diff};
use crate::paths::{is_component, is_within, resolved, BUILD_DIR};
use crate::plan::{ceiling_problems, retarget, NameCheck, Origin, Plan, PlannedFile, Totals};
use crate::watch::Watch;
use crate::{dlc, left_out, oiv, readers};

/// What follows `.<resource>` in the hidden file that says OpenRage built that resource.
const MARKER_SUFFIX: &str = ".openrage-build";

/// How a build makes the file a source folder ships: cook the folder (first) into the scratch folder (second) and say where the file is.
///
/// This crate knows no kind of source, so the editor and the command line hand it in.
pub type Cook<'a> = dyn Fn(&Path, &Path) -> Result<PathBuf, Vec<String>> + 'a;

/// A build that was written, verified and swapped in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// The resource, where the build left it.
    pub folder: PathBuf,
    /// The build it replaced, kept beside it, when there was one.
    pub previous: Option<PathBuf>,
    pub verification: Verification,
    /// What differs from the build it replaced, by stable id; none when that build recorded no items.
    pub since: Option<Diff>,
    /// What a file dropped on its way to the output's target, because the target has no field for it.
    pub losses: Vec<Loss>,
    /// The sources the build cooked and shipped, in path order.
    pub cooked: Vec<Cooked>,
    /// The sources of the game's own content that the cook left as the game has them, which the build did not ship.
    pub unchanged: Vec<Unchanged>,
    /// What the build wrote: the plan's files with each cooked file at the size it came out.
    pub written: Totals,
}

/// A file the build made by cooking a source folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cooked {
    /// The file, by its path in the resource.
    pub path: String,
    pub source: PathBuf,
    pub bytes: u64,
}

/// A source of the game's own content whose cook is the file the game has, so the build ships nothing of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unchanged {
    /// The file it would have been, by its path in the resource.
    pub path: String,
    pub source: PathBuf,
}

/// The values of one file that its target has no field for, each as the retarget words it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loss {
    /// The file, by its path in the resource.
    pub path: String,
    pub dropped: Vec<String>,
}

/// What reading a written folder back found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verification {
    /// Files compared with the plan, byte for byte.
    pub checked: usize,
    /// Every file a reader of OpenRage opened, with the reader that did; never cut short.
    pub reloaded: Vec<Reloaded>,
    /// The files locked by a server's asset escrow, by their path in the resource: equal to what the plan copies, and opened by no reader.
    pub locked: Vec<String>,
    /// One sentence for each file that is missing, differs, is not in the plan or does not reload.
    pub problems: Vec<String>,
    /// The names the game resolves once that the plan checked, and the ones held twice.
    pub names: NameCheck,
    /// What the RSC7 files take once the game streams them in, against the build's ceiling.
    pub memory: Memory,
    /// Whether a second build wrote the same bytes, which is made on request only.
    pub reproducible: Reproducible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reloaded {
    pub path: String,
    pub reader: String,
    /// For a metadata document, the elements of the arrays its root holds.
    pub entries: Option<usize>,
}

/// The metadata documents a reader opened under one name, such as `CPedVariationInfo (RBF)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataCount {
    pub reader: String,
    pub documents: usize,
    pub entries: usize,
}

/// The streaming memory of the RSC7 files a build wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Memory {
    /// The RSC7 files counted.
    pub files: usize,
    /// The sum of their virtual and physical page sizes, from their headers.
    pub bytes: u64,
    /// The build's `limits` entry `memory`, none when it states none.
    pub ceiling: Option<u64>,
}

/// Whether building the plan a second time into a scratch folder wrote the same bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Reproducible {
    #[default]
    NotRun,
    /// The second build wrote the same `files`, byte for byte.
    Yes { files: usize },
    /// One sentence for each file that differs, is missing or is extra, or the reason the second build stopped.
    No { differences: Vec<String> },
}

impl Reproducible {
    /// What the line says after `Reproducible:`.
    pub fn label(&self) -> String {
        match self {
            Self::NotRun => "not run".to_string(),
            Self::Yes { files } => format!("yes, {files} file(s) identical"),
            Self::No { differences } => format!("no, {} difference(s)", differences.len()),
        }
    }
}

impl Verification {
    /// The files compared with the plan that no reader reads.
    pub fn without_reader(&self) -> usize {
        self.checked.saturating_sub(self.reloaded.len())
    }

    /// The count the verification starts with: files compared, reloaded and read by none.
    pub fn reload_line(&self) -> String {
        format!(
            "{} file(s) compared with the plan, {} reloaded through a reader, {} with no reader",
            self.checked,
            self.reloaded.len(),
            self.without_reader()
        )
    }

    /// The metadata documents reloaded, by what they are, in the order of their names.
    pub fn metadata(&self) -> Vec<MetadataCount> {
        let mut counts: BTreeMap<&str, MetadataCount> = BTreeMap::new();
        for reloaded in &self.reloaded {
            let Some(entries) = reloaded.entries else {
                continue;
            };
            let count = counts
                .entry(&reloaded.reader)
                .or_insert_with(|| MetadataCount {
                    reader: reloaded.reader.clone(),
                    documents: 0,
                    entries: 0,
                });
            count.documents += 1;
            count.entries += entries;
        }
        counts.into_values().collect()
    }
}

/// The `limits` entry that holds a build's ceiling for the memory its RSC7 files take once streamed in.
pub const MEMORY_LIMIT: &str = "memory";

/// Build `plan` again into a scratch folder beside the build and say whether it wrote what `built` did, byte for byte.
///
/// ⚠ The live build is not touched: the second one is written, read back and compared in a hidden
/// `.<resource>.again` folder and removed. A cook that stamps the time or a random value into its
/// file is what this finds. It asks for a second cook of every source, so it is made on request.
pub fn check_reproducible(
    plan: &Plan,
    built: &mut Built,
    root: &Path,
    remap: &ParamRemap,
    cook: Option<&Cook>,
) {
    built.verification.reproducible = match again(plan, &built.folder, root, remap, cook) {
        Ok(differences) if differences.is_empty() => Reproducible::Yes {
            files: files_under(&built.folder).len(),
        },
        Ok(differences) => Reproducible::No { differences },
        Err(e) => Reproducible::No {
            differences: vec![format!("the second build stopped: {e}")],
        },
    };
}

/// The differences between the build in `live` and a second build of `plan`, which is written under `root` and removed.
fn again(
    plan: &Plan,
    live: &Path,
    root: &Path,
    remap: &ParamRemap,
    cook: Option<&Cook>,
) -> Result<Vec<String>, BuildError> {
    let second = root.join(format!(".{}.again", plan.resource));
    let scratch = root.join(format!(".{}.cook", plan.resource));
    remove_if_present(&second)?;
    remove_if_present(&scratch)?;
    let watch = Watch::default();
    let compared = cooking(plan, &scratch, cook, &watch)
        .and_then(|cooking| write_and_read_back(&cooking.plan, &second, remap, &|_| {}, &watch))
        .map(|_| differences(live, &second));
    let _ = std::fs::remove_dir_all(&scratch);
    let _ = std::fs::remove_dir_all(&second);
    compared
}

/// One sentence for each file that is in one of the folders only, or in both with other bytes.
fn differences(first: &Path, second: &Path) -> Vec<String> {
    let (one, two) = (files_under(first), files_under(second));
    let mut found = Vec::new();
    for path in &one {
        if !two.contains(path) {
            found.push(format!("`{path}` is not in the second build"));
        } else if std::fs::read(first.join(path)).ok() != std::fs::read(second.join(path)).ok() {
            found.push(format!("`{path}` differs between the two builds"));
        }
    }
    found.extend(
        two.iter()
            .filter(|path| !one.contains(path))
            .map(|path| format!("`{path}` is only in the second build")),
    );
    found
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("the plan cannot be built: {}", .0.join("; "))]
    Refused(Vec<String>),
    #[error("{what}: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{}` changed since the plan was made", .0.display())]
    SourceChanged(PathBuf),
    #[error("a source cannot be cooked: {}", .0.join("; "))]
    Cooking(Vec<String>),
    #[error("`{file}` cannot be moved to the build's target: {why}")]
    Retargeting { file: String, why: String },
    #[error("what the build wrote does not read back: {}", .0.problems.join("; "))]
    Unverified(Box<Verification>),
    /// Somebody asked it to stop, and the live build is as it was.
    #[error("the build was stopped")]
    Stopped,
}

/// Write `plan` as `root/<resource>`, leaving the build it replaces beside it as a hidden `.previous` folder.
///
/// The files go to a fresh hidden `.new` folder first and are read back there, so any failure
/// removes that folder and leaves the live build as it was. The swap itself is two renames, and
/// the second one failing puts the first back. The items the plan carries are recorded after the
/// swap, which is what the next build reports its difference from.
///
/// ⚠ A plan is data a caller may have built by hand, so this is where a path that climbs out of
/// the resource is refused: nothing is written before it is.
///
/// ⚠ A build deletes and replaces folders, so it refuses to put any of the three where the project
/// is, inside it (but for `build/`) or where a file it copies is, and to replace a folder that
/// OpenRage did not build: a hidden `.<resource>.openrage-build` beside the live folder says it did.
///
/// ⚠ This cooks no source: a plan that holds one is refused, since no cooker was handed in (see [`build_with`]).
pub fn build(plan: &Plan, root: &Path) -> Result<Built, BuildError> {
    build_with(plan, root, &ParamRemap::default(), None)
}

/// [`build`], respelling the material parameters of a drawable that moves to another target through `remap`, and cooking the source folders of the plan through `cook`.
///
/// ⚠ With no database in it, every parameter of such a drawable is dropped and named in [`Built::losses`].
///
/// ⚠ **The cook comes before anything is written.** Each source is cooked into a hidden `.cook` folder
/// beside the build, and one that fails stops the build with the live one untouched and every failure
/// named. A source of the game's own content whose cook is the game's file is not shipped
/// ([`Built::unchanged`]) unless the plan says to ship it, and a cooked file moves to the plan's
/// target as a retarget does. The `.cook` folder is removed whatever happens.
pub fn build_with(
    plan: &Plan,
    root: &Path,
    remap: &ParamRemap,
    cook: Option<&Cook>,
) -> Result<Built, BuildError> {
    build_observed(plan, root, remap, cook, &|_| {})
}

/// The steps of a build after its plan, in the order it takes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuildStep {
    Cook,
    Write,
    ReadBack,
    Swap,
}

impl BuildStep {
    pub const ALL: [BuildStep; 4] = [Self::Cook, Self::Write, Self::ReadBack, Self::Swap];
}

/// What a build calls as it enters each of its steps.
pub type Observe<'a> = dyn Fn(BuildStep) + 'a;

/// [`build_with`], telling `observe` each step as the build enters it; a build that stops reports no step it never reached.
pub fn build_observed(
    plan: &Plan,
    root: &Path,
    remap: &ParamRemap,
    cook: Option<&Cook>,
    observe: &Observe,
) -> Result<Built, BuildError> {
    build_watched(plan, root, remap, cook, observe, &Watch::default())
}

/// [`build_observed`], saying through `watch` what file it is on and how many bytes of the plan are done, and stopping when `watch` is cancelled.
///
/// ⚠ A build stopped before the folders are swapped removes the folder it was writing and returns [`BuildError::Stopped`]; the build it would have replaced is untouched.
pub fn build_watched(
    plan: &Plan,
    root: &Path,
    remap: &ParamRemap,
    cook: Option<&Cook>,
    observe: &Observe,
    watch: &Watch,
) -> Result<Built, BuildError> {
    let why = refusals(plan, root);
    if !why.is_empty() {
        return Err(BuildError::Refused(why));
    }
    let Places {
        live,
        staging,
        previous,
        marker,
        scratch,
    } = Places::of(&plan.resource, root);

    let since = diff::since_last_build(plan, root);
    std::fs::create_dir_all(root).map_err(io_error(format!("creating {}", root.display())))?;
    // ⚠ Folders an earlier build died in.
    remove_if_present(&staging)?;
    remove_if_present(&scratch)?;
    observe(BuildStep::Cook);
    let written = cooking(plan, &scratch, cook, watch).and_then(|cooking| {
        observe(BuildStep::Write);
        let (verification, losses) =
            write_and_read_back(&cooking.plan, &staging, remap, observe, watch)?;
        std::fs::write(
            &marker,
            format!(
                "OpenRage built the folder `{}` beside this file and replaces it at every build.\n",
                plan.resource
            ),
        )
        .map_err(io_error(format!("writing {}", marker.display())))?;
        watch.go_on()?;
        observe(BuildStep::Swap);
        let kept = swap(&staging, &live, &previous)?;
        Ok((verification, losses, kept, cooking))
    });
    let _ = std::fs::remove_dir_all(&scratch);
    match written {
        // The record follows the swap, so a build that failed leaves the last good one's.
        Ok((verification, losses, kept, cooking)) => {
            diff::record(root, &plan.resource, &plan.items)
                .map_err(io_error(format!(
                    "the build is in place, but recording its items in {} failed",
                    root.display()
                )))
                .map(|()| Built {
                    folder: live,
                    previous: kept.then_some(previous),
                    verification,
                    since,
                    losses,
                    written: cooking.plan.totals,
                    cooked: cooking.cooked,
                    unchanged: cooking.unchanged,
                })
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// Build `plans`, the resources of one split output, and say what was built and what stopped the rest.
///
/// Every plan is checked before the first is written, so a plan or a folder that any resource
/// refuses stops the whole set with nothing written. What goes wrong while one is being written
/// (a source that changed, an I/O error, a file that does not read back) stops there, and the
/// resources before it stay built: the swaps are one per resource, not one for the set.
pub fn build_all(plans: &[Plan], root: &Path) -> (Vec<Built>, Option<BuildError>) {
    build_all_with(plans, root, &ParamRemap::default(), None)
}

/// [`build_all`], respelling parameters through `remap` and cooking sources through `cook` as [`build_with`] does.
pub fn build_all_with(
    plans: &[Plan],
    root: &Path,
    remap: &ParamRemap,
    cook: Option<&Cook>,
) -> (Vec<Built>, Option<BuildError>) {
    build_all_observed(plans, root, remap, cook, &|_| {})
}

/// [`build_all_with`], telling `observe` the steps of each resource as [`build_observed`] does.
pub fn build_all_observed(
    plans: &[Plan],
    root: &Path,
    remap: &ParamRemap,
    cook: Option<&Cook>,
    observe: &Observe,
) -> (Vec<Built>, Option<BuildError>) {
    build_all_watched(plans, root, remap, cook, observe, &Watch::default())
}

/// [`build_all_observed`], watched as [`build_watched`] is: the one being written is stopped, and the ones after it are not begun.
pub fn build_all_watched(
    plans: &[Plan],
    root: &Path,
    remap: &ParamRemap,
    cook: Option<&Cook>,
    observe: &Observe,
    watch: &Watch,
) -> (Vec<Built>, Option<BuildError>) {
    let why: Vec<String> = plans.iter().flat_map(|plan| refusals(plan, root)).collect();
    if !why.is_empty() {
        return (Vec::new(), Some(BuildError::Refused(why)));
    }
    let mut built = Vec::new();
    for plan in plans {
        match build_watched(plan, root, remap, cook, observe, watch) {
            Ok(one) => built.push(one),
            Err(e) => return (built, Some(e)),
        }
    }
    (built, None)
}

/// What cooking the sources of a plan left: the plan to write, and what the cook did.
struct Cooking<'a> {
    /// The plan with each cooked file a copy of the cook's output, and each unchanged one left out; the plan itself when it holds no source.
    plan: Cow<'a, Plan>,
    cooked: Vec<Cooked>,
    unchanged: Vec<Unchanged>,
}

/// Cook every source folder `plan` holds into `scratch`, and make the plan that writes what came out.
///
/// ⚠ Every kind of output leaves out a source whose cook is a file of the game's own, unless the plan
/// says to ship it. A pack and a package name each of their files when they are planned, so what is
/// left out is also taken out of the registrations and the manifest, which are written again
/// ([`crate::left_out`]). A file that has to move to the target is not the game's file in that
/// layout, so it ships.
fn cooking<'a>(
    plan: &'a Plan,
    scratch: &Path,
    cook: Option<&Cook>,
    watch: &Watch,
) -> Result<Cooking<'a>, BuildError> {
    if !plan
        .files
        .iter()
        .any(|f| matches!(f.origin, Origin::Cook { .. }))
    {
        return Ok(Cooking {
            plan: Cow::Borrowed(plan),
            cooked: Vec::new(),
            unchanged: Vec::new(),
        });
    }
    let Some(cook) = cook else {
        return Err(BuildError::Cooking(vec![
            "the plan holds source folders and this build was handed no cooker to cook them"
                .to_string(),
        ]));
    };
    std::fs::create_dir_all(scratch)
        .map_err(io_error(format!("creating {}", scratch.display())))?;

    let skippable = !plan.ship_unchanged;
    let mut files: Vec<PlannedFile> = Vec::with_capacity(plan.files.len());
    let (mut cooked, mut unchanged, mut problems) = (Vec::new(), Vec::new(), Vec::new());
    let mut made: Vec<(PathBuf, &Path)> = Vec::new();
    for file in &plan.files {
        let Origin::Cook { source, game, .. } = &file.origin else {
            files.push(file.clone());
            continue;
        };
        watch.go_on()?;
        watch.progress.now_on(source.display().to_string());
        let output = match cook(source, scratch) {
            Ok(output) => output,
            Err(why) => {
                problems.extend(
                    why.into_iter()
                        .map(|w| format!("{}: {w}", source.display())),
                );
                continue;
            }
        };
        // ⚠ Two folders with one id cook into one slot, and the second would be the first's file.
        if let Some((_, first)) = made.iter().find(|(held, _)| *held == output) {
            problems.push(format!(
                "`{}` and `{}` cook into one file, so they have one id in their asset.json: give one a new id",
                first.display(),
                source.display()
            ));
            continue;
        }
        made.push((output.clone(), source));
        let data = std::fs::read(&output)
            .map_err(io_error(format!("reading the cooked {}", output.display())))?;
        // ⚠ A file with no layout per target (a metadata file) fits every target, so it can be unchanged from the game in one.
        let layout_fits = plan.target.is_none_or(|t| {
            !readers::has_layout(&file.path)
                || readers::layout(&file.path, &data).is_ok_and(|l| l == t.generation())
        });
        if skippable && layout_fits && game.contains(&digest(&data)) {
            unchanged.push(Unchanged {
                path: file.path.clone(),
                source: source.clone(),
            });
            continue;
        }
        let bytes = data.len() as u64;
        cooked.push(Cooked {
            path: file.path.clone(),
            source: source.clone(),
            bytes,
        });
        let mut shipped = PlannedFile {
            path: file.path.clone(),
            origin: Origin::Copy {
                from: output,
                bytes,
            },
        };
        if plan.target.is_some() {
            retarget(&mut shipped);
        }
        files.push(shipped);
    }
    if !problems.is_empty() {
        return Err(BuildError::Cooking(problems));
    }

    let mut written = plan.clone();
    written.files = files;
    if !unchanged.is_empty() {
        let gone = unchanged.iter().map(|u| u.path.clone()).collect();
        left_out::rewrite(&mut written, &gone).map_err(|why| BuildError::Refused(vec![why]))?;
    }
    left_out::count(&mut written);
    // The plan counted each cook as the payload it is made from; what came out may be over a ceiling.
    let over = ceiling_problems(&written);
    if !over.is_empty() {
        return Err(BuildError::Refused(over));
    }
    Ok(Cooking {
        plan: Cow::Owned(written),
        cooked,
        unchanged,
    })
}

/// The error for a source folder that reached a writer, which only a plan nobody cooked can hold.
pub(crate) fn uncooked(path: &str) -> BuildError {
    BuildError::Cooking(vec![format!(
        "`{path}` is a source folder that was not cooked"
    )])
}

/// Where a build of one resource puts things under the build root.
struct Places {
    live: PathBuf,
    staging: PathBuf,
    previous: PathBuf,
    marker: PathBuf,
    /// Where the sources are cooked to before anything is written.
    scratch: PathBuf,
}

impl Places {
    fn of(resource: &str, root: &Path) -> Self {
        Self {
            live: root.join(resource),
            staging: root.join(format!(".{resource}.new")),
            previous: root.join(format!(".{resource}.previous")),
            marker: root.join(format!(".{resource}{MARKER_SUFFIX}")),
            scratch: root.join(format!(".{resource}.cook")),
        }
    }
}

/// One sentence for each reason `plan` may not be built into `root`, found before anything is written.
fn refusals(plan: &Plan, root: &Path) -> Vec<String> {
    if !plan.is_buildable() {
        return plan.problems.clone();
    }
    let escaping: Vec<String> = plan
        .files
        .iter()
        .filter(|f| !f.path.split('/').all(is_component))
        .map(|f| format!("`{}` is not a path inside the resource", f.path))
        .chain(
            (!is_name(&plan.resource)).then(|| format!("`{}` is not a folder name", plan.resource)),
        )
        .collect();
    if !escaping.is_empty() {
        return escaping;
    }
    let at = Places::of(&plan.resource, root);
    let mut dangers = endangered(plan, [&at.live, &at.staging, &at.previous, &at.scratch]);
    if at.live.exists() && !(at.live.is_dir() && at.marker.is_file()) {
        dangers.push(format!(
            "`{}` is there and OpenRage did not build it, so a build does not replace it: move it or build elsewhere",
            at.live.display()
        ));
    }
    dangers
}

/// One sentence for each of `places` that is, holds or sits in the project, or is or holds a file the plan copies.
///
/// The project's `build/` folder is inside the project and is the one place in it that is allowed:
/// a plan never reads it back as content.
fn endangered<'a>(plan: &Plan, places: impl IntoIterator<Item = &'a PathBuf>) -> Vec<String> {
    let project = resolved(&plan.project_folder);
    let builds = resolved(&plan.project_folder.join(BUILD_DIR));
    let copied: Vec<PathBuf> = plan
        .files
        .iter()
        .filter_map(|f| match &f.origin {
            Origin::Copy { from, .. } | Origin::Retarget { from, .. } => Some(resolved(from)),
            Origin::Cook { source, .. } => Some(resolved(source)),
            Origin::Document(_) | Origin::Generated(_) => None,
        })
        .collect();
    let mut found = Vec::new();
    for place in places {
        let at = resolved(place);
        let in_builds = is_within(&at, &builds) && !is_within(&builds, &at);
        if is_within(&project, &at) || (is_within(&at, &project) && !in_builds) {
            found.push(format!(
                "`{}` is the project's folder, holds it or is inside it, and a build replaces and deletes there: build into `{BUILD_DIR}/` or outside the project",
                place.display()
            ));
        } else if copied
            .iter()
            .any(|from| is_within(from, &at) || is_within(&at, from))
        {
            found.push(format!(
                "`{}` is or holds a file the build copies, and a build replaces and deletes there",
                place.display()
            ));
        }
    }
    found
}

/// Put `staging` where `live` is, keeping what was there as `previous`; whether anything was.
fn swap(staging: &Path, live: &Path, previous: &Path) -> Result<bool, BuildError> {
    let kept = live.exists();
    if kept {
        remove_if_present(previous)?;
        std::fs::rename(live, previous).map_err(io_error(format!(
            "keeping the previous build as {}",
            previous.display()
        )))?;
    }
    if let Err(e) = std::fs::rename(staging, live) {
        if kept {
            let _ = std::fs::rename(previous, live);
        }
        return Err(io_error(format!("swapping in {}", live.display()))(e));
    }
    Ok(kept)
}

fn write_and_read_back(
    plan: &Plan,
    staging: &Path,
    remap: &ParamRemap,
    observe: &Observe,
    watch: &Watch,
) -> Result<(Verification, Vec<Loss>), BuildError> {
    std::fs::create_dir_all(staging)
        .map_err(io_error(format!("creating {}", staging.display())))?;
    let losses = match &plan.kind {
        OutputKind::FivemResource => write_folder(plan, staging, remap, watch)?,
        OutputKind::SingleplayerDlc => dlc::write(plan, staging, remap, watch)?,
        OutputKind::OivPackage => oiv::write(plan, staging, remap, watch)?,
        OutputKind::Unknown(kind) => {
            return Err(BuildError::Refused(vec![format!(
                "the build is of kind `{kind}`, which this version of OpenRage cannot make"
            )]))
        }
    };
    observe(BuildStep::ReadBack);
    watch.go_on()?;
    let verification = verify_watched(plan, staging, watch);
    watch.go_on()?;
    if verification.problems.is_empty() {
        Ok((verification, losses))
    } else {
        Err(BuildError::Unverified(Box::new(verification)))
    }
}

/// The files of a FiveM resource, each copied, written or moved to the output's target.
fn write_folder(
    plan: &Plan,
    staging: &Path,
    remap: &ParamRemap,
    watch: &Watch,
) -> Result<Vec<Loss>, BuildError> {
    let mut losses = Vec::new();
    let mut done = 0;
    for file in &plan.files {
        watch.go_on()?;
        watch.at(&file.path, done, plan.totals.bytes);
        done += file.bytes();
        let dest = staging.join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(io_error(format!("creating {}", parent.display())))?;
        }
        match &file.origin {
            Origin::Copy { from, bytes } => {
                let copied = std::fs::copy(from, &dest).map_err(io_error(format!(
                    "copying {} to {}",
                    from.display(),
                    file.path
                )))?;
                if copied != *bytes {
                    return Err(BuildError::SourceChanged(from.clone()));
                }
            }
            Origin::Document(text) => {
                std::fs::write(&dest, text).map_err(io_error(format!("writing {}", file.path)))?
            }
            Origin::Retarget { from, bytes } => {
                let (data, loss) = moved(plan, &file.path, from, *bytes, remap)?;
                std::fs::write(&dest, data).map_err(io_error(format!("writing {}", file.path)))?;
                losses.extend(loss);
            }
            Origin::Generated(bytes) => {
                std::fs::write(&dest, bytes).map_err(io_error(format!("writing {}", file.path)))?
            }
            Origin::Cook { .. } => return Err(uncooked(&file.path)),
        }
    }
    watch.all(plan.totals.bytes);
    Ok(losses)
}

/// The bytes of `from`, a source of `bytes` that the plan ships as `path` in its target, and what moving it dropped.
///
/// ⚠ A file already in the target's layout is copied exactly, and one the other layout cannot
/// describe stops the build, since carrying it as it is would ship bytes that mean something else.
pub(crate) fn moved(
    plan: &Plan,
    path: &str,
    from: &Path,
    bytes: u64,
    remap: &ParamRemap,
) -> Result<(Vec<u8>, Option<Loss>), BuildError> {
    let data = std::fs::read(from).map_err(io_error(format!("reading {}", from.display())))?;
    if data.len() as u64 != bytes {
        return Err(BuildError::SourceChanged(from.to_path_buf()));
    }
    // ⚠ No layout can be read off a file an asset escrow locks, so there is none to move it from: it ships as it came.
    if data.starts_with(crate::resource::ESCROW_MARKER) {
        return Ok((data, None));
    }
    let target = plan.target.map(|t| t.generation());
    if readers::layout(path, &data).ok() == target {
        return Ok((data, None));
    }
    let moved = retarget_file(path, &data, remap).map_err(|why| BuildError::Retargeting {
        file: path.to_string(),
        why,
    })?;
    let loss = (!moved.report.0.is_empty()).then(|| Loss {
        path: path.to_string(),
        dropped: moved.report.0.iter().map(ToString::to_string).collect(),
    });
    Ok((moved.bytes, loss))
}

/// What a clean removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub removed: Vec<PathBuf>,
}

/// The resources under `root` that a build of OpenRage made, found by their hidden markers, in name order.
///
/// A resource a project no longer declares, or an output that split into `<name>_1` and `<name>_2`, is
/// found here and by no output's name, so this is what a clean of the whole build folder walks.
pub fn built_resources(root: &Path) -> Result<Vec<String>, BuildError> {
    let reading = || format!("reading {}", root.display());
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_error(reading())(e)),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_error(reading()))?;
        let file_name = entry.file_name();
        let resource = file_name
            .to_str()
            .and_then(|name| name.strip_prefix('.'))
            .and_then(|name| name.strip_suffix(MARKER_SUFFIX));
        if let Some(resource) = resource.filter(|name| is_name(name) && entry.path().is_file()) {
            found.push(resource.to_string());
        }
    }
    found.sort();
    Ok(found)
}

/// Remove what a build of `resource` left under `root`: the resource, the build it replaced, a build that died midway, the record of its items and the marker.
///
/// ⚠ A clean deletes folders, so it removes the resource only where the hidden marker says OpenRage
/// built it, and never anything else under `root`. The record goes too, or the next plan would
/// report its difference from a build that is no longer there.
pub fn clean(root: &Path, resource: &str) -> Result<Cleaned, BuildError> {
    if !is_name(resource) {
        return Err(BuildError::Refused(vec![format!(
            "`{resource}` is not a folder name"
        )]));
    }
    let live = root.join(resource);
    let marker = root.join(format!(".{resource}{MARKER_SUFFIX}"));
    if live.exists() && !(live.is_dir() && marker.is_file()) {
        return Err(BuildError::Refused(vec![format!(
            "`{}` is there and OpenRage did not build it, so a clean leaves it",
            live.display()
        )]));
    }
    // ⚠ With no marker, the hidden folders beside it are nobody's to remove either.
    if !marker.is_file() {
        return Ok(Cleaned::default());
    }
    let mut cleaned = Cleaned::default();
    for place in [
        live,
        root.join(format!(".{resource}.previous")),
        root.join(format!(".{resource}.new")),
        root.join(format!(".{resource}.cook")),
    ] {
        if place.exists() {
            std::fs::remove_dir_all(&place)
                .map_err(io_error(format!("removing {}", place.display())))?;
            cleaned.removed.push(place);
        }
    }
    // ⚠ The marker last, so a clean that stopped midway can be run again.
    for file in [diff::record_path(root, resource), marker] {
        if file.exists() {
            std::fs::remove_file(&file)
                .map_err(io_error(format!("removing {}", file.display())))?;
            cleaned.removed.push(file);
        }
    }
    Ok(cleaned)
}

/// Read `folder` back against `plan`: every file present, equal to what the plan names, and reloaded where a reader reads it.
///
/// A file the plan retargets is not equal to its source, so it is checked to be in the target's
/// layout and to reload. A DLC pack and a package are read as the archive they are.
pub fn verify(plan: &Plan, folder: &Path) -> Verification {
    verify_watched(plan, folder, &Watch::default())
}

/// [`verify`], saying how many bytes of the plan are read back and giving up when `watch` is cancelled.
fn verify_watched(plan: &Plan, folder: &Path, watch: &Watch) -> Verification {
    // ⚠ The plan as made names the sources a build left out; a pack and a package are read against what they hold.
    let plan = &*left_out::as_built(plan, folder);
    let mut found = match &plan.kind {
        OutputKind::FivemResource => verify_folder(plan, folder, watch),
        OutputKind::SingleplayerDlc => dlc::verify(plan, folder),
        OutputKind::OivPackage => oiv::verify(plan, folder),
        OutputKind::Unknown(kind) => {
            return Verification {
                problems: vec![format!(
                    "the build is of kind `{kind}`, which this version of OpenRage cannot read back"
                )],
                ..Verification::default()
            }
        }
    };
    found.names = plan.names.clone();
    found.memory.ceiling = (plan.budgets.iter())
        .find(|b| b.name == MEMORY_LIMIT)
        .and_then(|b| b.ceiling);
    if let Some(max) = found.memory.ceiling.filter(|max| found.memory.bytes > *max) {
        found.problems.push(format!(
            "the files hold an estimated {} bytes of streaming memory, over the {max} that build `{}` allows",
            found.memory.bytes, plan.output
        ));
    }
    found
}

fn verify_folder(plan: &Plan, folder: &Path, watch: &Watch) -> Verification {
    let mut found = Verification::default();
    let mut done = 0;
    for file in &plan.files {
        if watch.cancel.is_set() {
            return found;
        }
        watch.at(&file.path, done, plan.totals.bytes);
        done += file.bytes();
        let read = std::fs::read(folder.join(&file.path));
        // ⚠ A source of the game's own content that the cook left as the game has it is not in the build, and the plan alone cannot say it was that.
        let left_out = matches!(&file.origin, Origin::Cook { game, .. } if !game.is_empty())
            && !plan.ship_unchanged
            && read
                .as_ref()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
        if left_out {
            continue;
        }
        found.checked += 1;
        match read {
            Ok(written) => check_file(&mut found, plan, file, &written),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                found.problems.push(format!("`{}` is missing", file.path));
            }
            Err(e) => found
                .problems
                .push(format!("`{}` cannot be read: {e}", file.path)),
        }
    }
    watch.all(plan.totals.bytes);
    for path in files_under(folder) {
        if !plan.files.iter().any(|f| f.path == path) {
            found
                .problems
                .push(format!("`{path}` is in the folder and not in the plan"));
        }
    }
    found
}

/// One written file against the plan: equal to what it names, or in the target's layout, and reloaded where a reader reads it.
pub(crate) fn check_file(
    found: &mut Verification,
    plan: &Plan,
    file: &PlannedFile,
    written: &[u8],
) {
    let locked = written.starts_with(crate::resource::ESCROW_MARKER);
    let copied = match &file.origin {
        Origin::Copy { from, .. } => Some(from),
        // ⚠ A locked file the build was to move to its target ships as it came, so it is compared as a copy is.
        Origin::Retarget { from, .. } if locked => Some(from),
        _ => None,
    };
    let planned = match (copied, &file.origin) {
        (Some(from), _) => match std::fs::read(from) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                found.problems.push(format!(
                    "`{}` cannot be read to compare: {e}",
                    from.display()
                ));
                return;
            }
        },
        (None, Origin::Document(text)) => Some(text.as_bytes().to_vec()),
        (None, Origin::Generated(bytes)) => Some(bytes.clone()),
        (None, _) => None,
    };
    if planned.is_some_and(|planned| written != planned) {
        found
            .problems
            .push(format!("`{}` differs from what the plan names", file.path));
        return;
    }
    // ⚠ A file an asset escrow locks is its author's as it came: equal to what the plan copies is all a build can say of it.
    if copied.is_some() && locked {
        found.locked.push(file.path.clone());
        return;
    }
    // ⚠ A file with no layout per target (a metadata file) has none to compare with the target's.
    let target = plan.target.filter(|_| readers::has_layout(&file.path));
    if let (Origin::Retarget { .. } | Origin::Cook { .. }, Some(target)) = (&file.origin, target) {
        match readers::layout(&file.path, written) {
            Ok(layout) if layout == target.generation() => {}
            Ok(layout) => {
                found.problems.push(format!(
                    "`{}` is laid out for {}, and the build ships {}",
                    file.path,
                    layout.edition_label(),
                    target.generation().edition_label()
                ));
                return;
            }
            Err(why) => {
                found
                    .problems
                    .push(format!("`{}` does not reload: {why}", file.path));
                return;
            }
        }
    }
    // ⚠ The pages the game allocates for it, named by its own header and so counted whether or not a reader opens it.
    if let Ok(seal) = Rsc7::peek(written) {
        found.memory.files += 1;
        found.memory.bytes += u64::from(seal.total_size());
    }
    match readers::reload(&file.path, written) {
        None => {}
        Some(Ok(reload)) => found.reloaded.push(Reloaded {
            path: file.path.clone(),
            reader: reload.reader,
            entries: reload.entries,
        }),
        Some(Err(why)) => found
            .problems
            .push(format!("`{}` does not reload: {why}", file.path)),
    }
}

/// Every file under `folder`, by its path inside it with `/`, in order.
pub(crate) fn files_under(folder: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                walk(base, &path, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(folder, folder, &mut out);
    out.sort();
    out
}

fn remove_if_present(folder: &Path) -> Result<(), BuildError> {
    if folder.exists() {
        std::fs::remove_dir_all(folder)
            .map_err(io_error(format!("removing {}", folder.display())))?;
    }
    Ok(())
}

fn io_error(what: String) -> impl FnOnce(std::io::Error) -> BuildError {
    move |source| BuildError::Io { what, source }
}
