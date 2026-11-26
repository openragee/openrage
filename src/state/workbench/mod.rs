//! What the Project workspace shows: the assets a project holds, its problems, its builds and the live recook of its sources.
//!
//! ⚠ **Nothing here draws, and nothing here touches the disk on the UI thread.** A scan, a cook and
//! a build each run on a worker and leave data; the panels read it. The container and the plan are
//! `or-project`'s and the cook is `or-source`'s, which this only calls.

mod alternates;
mod apply_template;
mod build_cooker;
mod build_edits;
mod builds;
mod commands;
mod cook;
mod doors;
mod dressed;
mod dressed_before;
mod edits;
mod findings;
mod game_collections;
mod geometry;
mod glance;
mod imports;
mod links;
mod maps;
mod maps_edit;
mod members;
mod originals;
mod output_sizes;
mod pack_import;
mod particles;
mod power;
mod problems_actions;
mod problems_view;
mod project_history;
mod read_only;
mod references;
mod reopen;
mod scan;
mod sharing;
mod strip;
mod tally;
mod wardrobe;
mod wardrobe_fix;
mod wardrobe_mesh;
mod wardrobe_operations;
mod wardrobe_places;
mod wardrobe_recovery;
mod wardrobe_select;
mod wardrobe_textures;
mod wardrobe_words;
mod watch;
mod weapons;
mod weapons_scene;
mod workspace_memory;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

pub use alternates::AlternatesEdit;
pub use build_edits::{NewBuild, KINDS};
#[cfg(test)]
pub use builds::BuiltSummary;
#[cfg(test)]
pub use builds::Job;
pub use builds::{verifies, BuildDot, Builds, BuiltPart, LastBuild, STEPS, VERIFY};
pub(super) use commands::is_a_place;
use cook::{Cook, Cooks};
pub use dressed::catalogue::TURNS;
pub use dressed::light;
#[cfg(test)]
pub(crate) use dressed::motion::tests::playing_on as a_clip_playing;
pub use dressed::motion::{PRESETS, SPEEDS};
#[cfg(test)]
pub(crate) use dressed::props::head_skeleton;
pub use dressed::view::ViewMode;
pub use dressed::{Dressed, Thumb, NO_GAME, TINTS};
pub use findings::{Finding, Fix, Missing, Place, Severity};
pub use game_collections::GameCollections;
pub use geometry::GeometryWork;
pub use glance::{Glance, Glimpse, Want};
pub use imports::{counts, said, ImportReport, ImportWork, NEEDS_A_PROJECT};
pub use maps::{
    held_by as maps_held, on_disk as maps_on_disk, pose_of_box, MapsRead, MapsWork, Pick, PickKind,
    PlanView, Preview,
};
#[cfg(test)]
pub use members::asks_for_a_folder;
pub use originals::Originals;
pub use particles::{
    held_by as particles_held, on_disk as particles_on_disk, EffectRow, ParticlesRead,
    ParticlesWork,
};
pub use power::{BulkDraft, BulkKind, CreditDraft};
#[cfg(test)]
pub use problems_view::Format as ProblemFormat;
pub use problems_view::{
    about as problem_about, content_of as problem_content, heading_words as problem_heading,
    rows as problem_rows, Group as ProblemGroup, Row as ProblemRow, Show as ProblemShow,
    Sort as ProblemSort, View as ProblemView,
};
pub use references::{Choice, Part, Picker, Wanting};
#[cfg(test)]
pub use reopen::destination;
pub use reopen::{asks_to_reopen, what_it_is, ReopenReport, ReopenWork};
pub use scan::{files_held, AssetRow, Kept, Resource, Scan, Seen};
pub use sharing::Concern;
#[cfg(test)]
pub use sharing::Kind as ConcernKind;
pub use strip::{split_strip, TabPlace};
pub use tally::{build_name, content_name, held, Counts, Scope};
pub use wardrobe::{LastGood, Pack, PackRead, WardrobeWork};
pub use wardrobe_fix::{Cut, Draft};
pub use wardrobe_mesh::{Before, Looked, MeshDesk};
pub use wardrobe_recovery::{Side, Versions};
pub use wardrobe_select::in_pack;
pub use wardrobe_textures::{MapsDraft, TextureDesk};
use watch::PackWatch;
pub use weapons::{held_by as weapons_held, on_disk as weapons_on_disk, WeaponsRead, WeaponsWork};
pub use weapons_scene::SceneWork;
pub(in crate::state) use workspace_memory::remembered as remembered_workspace;

/// How long a scan stays good while the Project workspace is on screen.
pub const RESCAN_AFTER: Duration = Duration::from_secs(3);

/// What the history keeps; older events go.
const HISTORY_KEPT: usize = 200;

/// Where an asset stands, as the table's first column says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetState {
    Ready,
    /// Open with edits the file on disk does not hold, which a build does not include.
    Unsaved,
    Missing,
    Folder,
    NotCooked,
    Cooked,
    Failed,
    Unreadable,
    /// A source whose check against the game found the game's file to be another one now.
    OriginalChanged,
    /// A garment whose worst finding is a warning.
    Warning,
    /// A garment whose worst finding is an error.
    Error,
}

impl AssetState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => tr!("Ready"),
            Self::Unsaved => tr!("Unsaved"),
            Self::Missing => tr!("Missing"),
            Self::Folder => tr!("Folder"),
            Self::NotCooked => tr!("Not compiled yet"),
            Self::Cooked => tr!("Compiled"),
            Self::Failed => tr!("Failed"),
            Self::Unreadable => tr!("Unreadable"),
            Self::OriginalChanged => tr!("The game's original changed"),
            Self::Warning => tr!("Warning"),
            Self::Error => tr!("Error"),
        }
    }

    pub fn is_problem(self) -> bool {
        matches!(
            self,
            Self::Missing
                | Self::Folder
                | Self::Failed
                | Self::Unreadable
                | Self::OriginalChanged
                | Self::Warning
                | Self::Error
        )
    }
}

/// Something that happened to the project in this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub at: SystemTime,
    pub text: String,
    /// The step of the project's history this event is, which a click goes back or forward to; none for an event that is no change.
    pub step: Option<u64>,
}

/// A question the project asks before it acts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    /// The Builds window: the build its form shows, and the build being added when one is.
    Builds {
        /// The id of the build shown; none when the project declares none.
        chosen: Option<String>,
        adding: Option<NewBuild>,
    },
    /// Remove every resource OpenRage built in `build/`, which are the names it found when this was asked, and the files and bytes in them.
    Clean {
        resources: Vec<String>,
        files: u64,
        bytes: u64,
    },
    /// Apply the fix of a finding of the clothing content `module`, after saying what it changes.
    Fix {
        module: String,
        finding: or_wardrobe::Finding,
    },
    /// Which drawable of the dictionary source at `key` the model in `file` goes into; `chosen` indexes `entries`.
    ReplaceGeometry {
        key: String,
        file: PathBuf,
        entries: Vec<String>,
        chosen: usize,
    },
    /// What replacing the geometry of the source `name` from `from` did: what the compile did and what the drawable had no place for, or why nothing changed.
    Geometry {
        name: String,
        from: String,
        replaced: bool,
        lines: Vec<String>,
    },
    /// What the texture optimiser is to do to the textures of `garments` (the whole pack where none) of the clothing content `module`.
    Optimise {
        module: String,
        garments: Option<Vec<String>>,
        draft: Draft,
    },
    /// What the maps are to be made of the textures of `garments` (the whole pack where none) of the clothing content `module`, and what they would be.
    Maps {
        module: String,
        garments: Option<Vec<String>>,
        draft: MapsDraft,
        lines: Vec<String>,
    },
    /// The plan of a texture budget for `garments` (the whole pack where none) of the clothing content `module`, which the window makes and shows before it is accepted.
    Budget {
        module: String,
        garments: Option<Vec<String>>,
    },
    /// What to do to the garments `ids` that the query `query` selects in the clothing content `module`, which the window says the effect of before Apply.
    Bulk {
        module: String,
        query: String,
        ids: Vec<String>,
        draft: BulkDraft,
        /// What `draft` would do, made when the draft changes and not again for each frame the window is drawn.
        shown: Option<std::sync::Arc<or_wardrobe::bulk::DryRun>>,
    },
    /// A name to keep the query `query` of the clothing content `module` under; `error` is why the last name was refused.
    SaveQuery {
        module: String,
        query: String,
        name: String,
        error: Option<String>,
        focus: bool,
    },
    /// The review of two versions of the pack of `module`: `from` and `to` are versions (none is the pack as it is) and `shown` is what differs between them.
    Review {
        module: String,
        from: Option<u64>,
        to: Option<u64>,
        shown: Option<std::sync::Arc<or_wardrobe::review::Review>>,
    },
    /// Who made the sources of garment `garment` of the clothing content `module`, and on what terms.
    Credit {
        module: String,
        garment: String,
        draft: CreditDraft,
        /// Whether the sources state a credit already, which can then be removed.
        had: bool,
    },
    /// Record the reason a finding of the clothing content `module` stands.
    Waive {
        module: String,
        finding: or_wardrobe::Finding,
        reason: String,
        focus: bool,
    },
    /// Record the reason a finding of a validator plugin, by its code, stands in this project.
    WaivePlugin {
        code: String,
        reason: String,
        focus: bool,
    },
    /// Write the files a validator plugin suggested with its finding, after listing them: `message` is the problem, `fix` what it suggests.
    PluginFix {
        name: String,
        code: String,
        message: String,
        fix: or_plugin::SuggestedFix,
    },
    /// The rules of the clothing content `module`, as the person changes them: `draft` is what Apply makes of them.
    Rules {
        module: String,
        draft: or_wardrobe::settings::Settings,
    },
    /// Models or textures dropped on a project that holds no clothing: make clothing for them, or import them as game files.
    DropClothes { paths: Vec<PathBuf> },
    /// A dropped folder that holds content (`kind`, in words): link it as that content, or import everything dropped as sources.
    DropContent {
        folder: PathBuf,
        kind: String,
        paths: Vec<PathBuf>,
    },
    /// Check before sharing, asked at `asked` and waiting for a scan that began after it, so the sources it lists are the folder's as it is now.
    SharingRead { asked: Instant },
    /// What Check before sharing found in the project folder; empty when it found nothing.
    Sharing { concerns: Vec<Concern> },
    /// A new source of `kind` is to be started empty, and the name is asked for; `error` is why the name last typed was no name.
    NewSource {
        kind: String,
        /// What the entry that asked is called: `Texture dictionary`.
        label: String,
        name: String,
        error: Option<String>,
        focus: bool,
    },
    /// What a content needs and does not own: the picker, which takes this project's contents, a file somewhere else or an id.
    Pick(Box<references::Picker>),
    /// A file of another place was chosen: import it into the project, or only reference it.
    ImportOrReference {
        wanting: references::Wanting,
        path: PathBuf,
        /// The project that holds it, when one does.
        whose: Option<String>,
    },
    /// Going back to the History step `step` takes back imports, and with them the source folders `sources` name: ask before removing them.
    RollBack {
        step: u64,
        label: String,
        sources: Vec<String>,
    },
    /// Whether the built pack `built` is made a project of sources in `name` inside the folder `place`; `pack` names the pack where it does not, and `error` is why the last answer was refused.
    Reopen {
        built: PathBuf,
        place: String,
        name: String,
        pack: String,
        error: Option<String>,
        focus: bool,
    },
}

/// What the scan counted when it was asked for, so a change in it asks for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signature {
    members: usize,
    open: usize,
}

#[derive(Default)]
pub struct Workbench {
    /// The project the rest describes.
    project: Option<PathBuf>,
    /// The right to write the open project, held for as long as this window has it open.
    project_lock: Option<or_project::lock::ProjectLock>,
    /// Why this window only reads the open project: another window holds it.
    held_by_another: Option<String>,
    /// The generation of the rows the members were last followed to, so a scan that changed none asks nothing.
    followed: Option<u64>,
    /// The workspace last seen on screen with this project open, which a change from is kept for the next opening.
    workspace_seen: Option<crate::ui::dock::Workspace>,
    /// The task screen the window was moved off because the project lost its last content of the kind, which it goes back to if the content returns while it is still where it was put.
    left_screen: Option<crate::ui::dock::Workspace>,
    /// What the project's Git block says its repository holds, read once for the project.
    commit: Option<or_source::git::Commit>,
    scan: Option<Scan>,
    scanning: Option<Receiver<Scan>>,
    wanted: bool,
    last_scan: Option<Instant>,
    signature: Option<Signature>,
    cooks: Option<Cooks>,
    /// The watch on the folders of the clothing packs, which stands in for the clock on the Wardrobe.
    pack_watch: Option<PackWatch>,
    builds: Builds,
    /// The member files open in a tab with edits not saved.
    unsaved: HashSet<PathBuf>,
    /// What the project file said that this build could not read.
    from_file: Vec<String>,
    findings: Vec<Finding>,
    /// The worst finding of each garment's row, by its key.
    garment_worst: HashMap<String, Severity>,
    findings_stale: bool,
    history: Vec<Event>,
    /// The steps of the project that Undo, Redo and a rollback move along.
    steps: project_history::Steps,
    /// Ceiling edits whose line in the history and rescan wait for the drag or the typing to end, by build and ceiling.
    unsaid: BTreeMap<String, String>,
    /// Bumped whenever the rows change, so a table filters again.
    generation: u64,
    /// The key of the selected asset; none selects the project itself.
    pub selected: Option<String>,
    /// The id of the content chosen in the Content panel, which Unlink acts on.
    pub selected_content: Option<String>,
    /// The id of the build the Build button builds; the first when none is chosen.
    pub selected_output: Option<String>,
    pub asking: Option<Question>,
    /// The quick look is open on the selected asset.
    pub quick_look: bool,
    /// The picture the quick look draws, read off the UI thread.
    pub glance: Glance,
    /// The imports of game files into the project: the one running and the last report.
    pub imports: ImportWork,
    /// A built pack made a project of sources: the one running and the last report.
    pub reopen: ReopenWork,
    /// The model being put into a source, if one is.
    pub geometry: GeometryWork,
    /// The checks of sources against the game: the one running and what the ones of this session found.
    originals: Originals,
    /// The Wardrobe workspace's selection, edits and drops.
    pub wardrobe: WardrobeWork,
    /// The Weapons workspace's selection and the changes Undo takes back.
    pub weapons: WeaponsWork,
    /// The Map workspace's selection, manipulator and the changes Undo takes back.
    pub maps: MapsWork,
    /// The Particle workspace's effect to show, while a project holds particles.
    pub particles: ParticlesWork,
    /// The collections the installed game's DLC adds, for the pack that replaces some.
    pub collections: GameCollections,
    /// Files a recook wrote that a tab with unsaved edits is still asking about, which the question words as a refresh from the source.
    pub refreshed: HashSet<PathBuf>,
}

impl Workbench {
    /// Start over for `project`, which is none when it was closed.
    pub(super) fn reset(&mut self, project: Option<PathBuf>) {
        // ⚠ The generation only grows: a table keeps what it filtered by it, and a new project that
        // started again at 0 would read the last project's filtered rows.
        let generation = self.generation + 1;
        // ⚠ So does the Wardrobe's: its list is cached by it in `state.panels`, which a project switch keeps.
        let wardrobe_generation = self.wardrobe.generation + 1;
        // ⚠ A build that is writing is not forgotten by moving on: its worker goes on, and forgetting it
        // would let a second one start over the same folder.
        let builds = std::mem::take(&mut self.builds).kept_while_running();
        // ⚠ A pack being reopened is not forgotten by moving on either: the folder it makes is not this project's.
        let reopen = std::mem::take(&mut self.reopen);
        // ⚠ The game's DLC list belongs to the install and not to the project, and reading it mounts the game.
        let collections = std::mem::take(&mut self.collections);
        *self = Self {
            project,
            wanted: true,
            generation,
            builds,
            reopen,
            collections,
            ..Self::default()
        };
        self.wardrobe.generation = wardrobe_generation;
    }

    /// Ask for the disk to be read again at the next poll.
    pub fn want_scan(&mut self) {
        self.wanted = true;
    }

    pub fn is_scanned(&self) -> bool {
        self.scan.is_some()
    }

    /// Whether the last scan read the disk as it is now: none is asked for or running.
    pub fn scan_is_current(&self) -> bool {
        self.scan.is_some() && self.scanning.is_none() && !self.wanted
    }

    pub fn rows(&self) -> &[AssetRow] {
        self.scan.as_ref().map_or(&[], |scan| &scan.rows)
    }

    /// The clothing content as the last scan read it.
    pub fn packs(&self) -> &[Pack] {
        self.scan.as_ref().map_or(&[], |scan| &scan.packs)
    }

    /// How many peds each addon ped content held when the last scan read it, or why its model could not be read.
    pub fn peds(&self) -> &[(String, Result<usize, String>)] {
        self.scan.as_ref().map_or(&[], |scan| &scan.peds)
    }

    pub fn row(&self, key: &str) -> Option<&AssetRow> {
        self.rows().iter().find(|row| row.key == key)
    }

    /// Changes when the rows do.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The walk for source folders stopped at its bounds.
    pub fn is_truncated(&self) -> bool {
        self.scan.as_ref().is_some_and(|scan| scan.truncated)
    }

    /// The plans of `output` as the last scan made them: one, or one for each resource it splits into.
    pub fn resources_of(&self, output: &str) -> &[Resource] {
        let planned = self
            .scan
            .as_ref()
            .and_then(|scan| scan.plans.iter().find(|planned| planned.output == output));
        planned.map_or(&[], |planned| planned.resources.as_slice())
    }

    /// What the files of the clothing content `module` weigh by garment and slot, as the options of the build `output` plan them.
    pub fn sizes_of(&self, output: &str, module: &str) -> Option<&or_wardrobe::sizes::Sizes> {
        let planned =
            (self.scan.as_ref()?.plans.iter()).find(|planned| planned.output == output)?;
        (planned.sizes.iter())
            .find(|(content, _)| content == module)
            .map(|(_, sizes)| sizes)
    }

    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// What is wrong with the asset `row`: the findings of its file or folder, or of the garment it stands for.
    pub fn findings_of<'a>(&'a self, row: &'a AssetRow) -> impl Iterator<Item = &'a Finding> {
        self.findings.iter().filter(move |f| match &f.place {
            Place::Asset { key, .. } => *key == row.key,
            Place::Garment {
                content,
                id: Some(id),
                ..
            } => scan::garment_key(content, id) == row.key,
            _ => false,
        })
    }

    /// The link of content `id` as the last scan showed it; none for content that is not a link.
    pub fn link(&self, id: &str) -> Option<&or_project::links::Link> {
        self.scan
            .as_ref()?
            .links
            .iter()
            .find(|link| link.module == id)
    }

    pub fn builds(&self) -> &Builds {
        &self.builds
    }

    /// What happened to the project this session, oldest first.
    pub fn history(&self) -> &[Event] {
        &self.history
    }

    /// Where `row` stands: what the disk showed, what the cook last said, and whether it is open with edits.
    pub fn state_of(&self, row: &AssetRow) -> AssetState {
        if row.kept == Kept::Pack {
            return match self.garment_worst.get(&row.key) {
                Some(Severity::Error) => AssetState::Error,
                Some(Severity::Warning) => AssetState::Warning,
                None => AssetState::Ready,
            };
        }
        match &row.seen {
            Seen::Missing => return AssetState::Missing,
            Seen::Folder => return AssetState::Folder,
            Seen::Unreadable(_) => return AssetState::Unreadable,
            Seen::Present | Seen::NotCooked | Seen::Cooked => {}
        }
        if row.kept == Kept::File {
            return if self.unsaved.contains(&row.path) {
                AssetState::Unsaved
            } else {
                AssetState::Ready
            };
        }
        let cooked = self.cooks.as_ref().and_then(|cooks| cooks.state(&row.path));
        if self.originals.is_changed(&row.path) && !matches!(cooked, Some(Cook::Failed(_))) {
            return AssetState::OriginalChanged;
        }
        match cooked {
            Some(Cook::Failed(_)) => AssetState::Failed,
            Some(Cook::Cooked) => AssetState::Cooked,
            None if row.seen == Seen::Cooked => AssetState::Cooked,
            None => AssetState::NotCooked,
        }
    }

    /// Whether the plan of `output` has a problem a build would stop on.
    pub fn is_blocked(&self, output: &str) -> bool {
        (self.resources_of(output).iter()).any(|resource| !resource.plan.is_buildable())
    }

    /// The state dot of `output`.
    pub fn dot(&self, output: &str) -> BuildDot {
        self.builds.dot(output, self.is_blocked(output))
    }

    /// How many problems the plans of `output` have, which a blocked build is refused for; said through [`Scope::Build`].
    fn plan_problems_of(&self, output: &str) -> usize {
        (self.resources_of(output).iter())
            .map(|resource| resource.plan.problems.len())
            .sum()
    }

    /// Take `last` as how `output` last ended.
    #[cfg(test)]
    pub fn keep_a_build(&mut self, output: &str, last: LastBuild) {
        self.builds.keep(output, last);
    }

    /// Read the builds against `sheet` in place of the acceptance sheet the tree ships.
    #[cfg(test)]
    pub fn read_builds_against(&mut self, sheet: or_project::acceptance::Sheet) {
        self.builds.read_against(sheet);
    }

    pub(in crate::state) fn note(&mut self, text: impl Into<String>) {
        self.history.push(Event {
            at: SystemTime::now(),
            text: text.into(),
            step: None,
        });
        if self.history.len() > HISTORY_KEPT {
            self.history.remove(0);
        }
    }

    /// What a scan left, once it has: the rows, and a watch on each source folder.
    fn take_scan(&mut self) {
        let Some(receiver) = &self.scanning else {
            return;
        };
        let Ok(scan) = receiver.try_recv() else {
            return;
        };
        self.scanning = None;
        if self.project.as_deref() != Some(scan.project.as_path()) {
            return;
        }
        if let Some(cooks) = self.cooks.as_mut() {
            for row in scan.rows.iter().filter(|row| row.kept == Kept::Source) {
                cooks.watch(&row.path);
            }
        }
        if self.scan.as_ref().map(|old| &old.rows) != Some(&scan.rows) {
            self.generation += 1;
        }
        if self.scan.as_ref().map(|old| &old.packs) != Some(&scan.packs) {
            self.wardrobe.generation += 1;
        }
        self.scan = Some(scan);
        self.follow_packs();
        self.findings_stale = true;
    }

    fn recompute_findings(&mut self) {
        self.findings_stale = false;
        self.findings = match &self.scan {
            Some(scan) => {
                let mut found = findings::collect(
                    scan,
                    self.cooks.as_ref(),
                    &self.unsaved,
                    &self.from_file,
                    &self.builds,
                );
                found.extend(findings::original_changes(scan, &self.originals));
                found
            }
            None => Vec::new(),
        };
        self.garment_worst.clear();
        for finding in &self.findings {
            if let Place::Garment {
                content,
                id: Some(id),
                ..
            } = &finding.place
            {
                let worst = self
                    .garment_worst
                    .entry(scan::garment_key(content, id))
                    .or_insert(finding.severity);
                *worst = (*worst).max(finding.severity);
            }
        }
    }

    /// The project file's own findings, kept so the Problems panel can list them.
    pub fn note_findings(&mut self, findings: Vec<String>) {
        self.from_file = findings;
        self.findings_stale = true;
    }
}

/// The folder a source's name is, for a sentence.
pub(super) fn leaf(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

impl Workbench {
    /// Stop the build being written and the run it is in.
    pub fn stop_building(&mut self) {
        self.builds.stop();
    }
}

impl Workbench {
    /// A file in a source folder was saved at `at`: its asset is cooked once the file has been quiet.
    pub fn file_saved(&mut self, path: PathBuf, at: Instant) {
        if let Some(cooks) = self.cooks.as_mut() {
            cooks.saved(path, at);
        }
    }

    /// Cook only what `file_saved` is told of, as a test that plays the watcher needs.
    #[cfg(test)]
    pub fn cook_only_what_is_saved(&mut self) {
        if let Some(cooks) = self.cooks.as_mut() {
            cooks.stop_listening();
        }
    }

    /// Whether every path `file_saved` was given has been cooked, or ignored, and its answer taken.
    #[cfg(test)]
    pub fn cooks_are_idle(&self) -> bool {
        self.cooks.as_ref().is_none_or(Cooks::is_idle)
    }
}
