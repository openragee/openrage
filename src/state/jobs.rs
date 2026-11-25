//! Every background job in one list: what it is doing, how far it has got, its Cancel, and what it left.
//!
//! ⚠ Each job stays in its own controller, which is the truth about whether it runs; the list
//! reads them every frame, so it never shows a job its controller has dropped.

use crate::state::Rpf9Document;

use std::path::{Path, PathBuf};

use super::{AppState, DocumentId};

/// What a background job is, as the list names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Extraction,
    Defragment,
    OverlayCopy,
    Search,
    FileIndex,
    Keys,
    /// The helper program baking the SPIR-V pack the viewport's game-shaders mode draws with.
    GamePackBake,
    NameScan,
    Load,
    Save,
    Conversion,
    /// A glTF export's write and Export as…'s: one model out to a file other tools open.
    Export,
    DlcPack,
    Gltf,
    Dependencies,
    Compare,
    /// A build of the open project: the files it plans, written to a fresh folder and read back.
    ProjectBuild,
    /// A project's files read after it opened: the scan that lists its content and counts its files.
    ProjectRead,
    /// Game files being decompiled into sources of the open project, off the UI thread.
    ProjectImport,
    /// A built pack being read into a new project folder of sources, off the UI thread.
    ProjectReopen,
    /// A GTA VI archive scanned for the bodies stored in the clear, a slice at a time.
    ArchiveScan,
    /// A nested archive's table read when its row is expanded.
    ArchiveTable,
    /// A GTA VI effect container read for the names of drawables' shaders.
    EffectsRead,
    /// The texture naming rules run over the names GTA VI drawables state.
    TextureNames,
    /// A scan's navigation cells read for those across a GTA VI cell's borders.
    NeighbourCells,
    /// A GTA VI archive's animation bodies read for the sets a clip dictionary names.
    SetSearch,
    /// A GTA VI archive's map types bodies read for the archetypes a map names.
    ArchetypeSearch,
    /// The index of a GTA VI install the World screen flies through, built on a worker.
    WorldIndex,
    /// A GTA VI archive's audio containers and bodies read for what audio game data links to.
    AudioLinks,
    /// A GTA VI audio stream's Opus decoded to play or export, or encoded from a WAV.
    OpusAudio,
    /// The coder's tables of GTA VI texture packs fitted from the packs of a folder.
    TextureCalibration,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Extraction => tr!("Extraction"),
            Self::Defragment => tr!("Defragment"),
            Self::OverlayCopy => tr!("Copy to the mods overlay"),
            Self::Search => tr!("Search"),
            Self::FileIndex => tr!("Game files"),
            Self::Keys => tr!("Game keys"),
            Self::GamePackBake => tr!("Game shader bake"),
            Self::NameScan => tr!("Game names"),
            Self::Load => tr!("Open"),
            Self::Save => tr!("Save"),
            Self::Conversion => tr!("Conversion"),
            Self::Export => tr!("Export"),
            Self::DlcPack => tr!("DLC pack"),
            Self::Gltf => tr!("glTF"),
            Self::Dependencies => tr!("Uses and Placed by"),
            Self::Compare => tr!("Comparison"),
            Self::ProjectBuild => tr!("Project build"),
            Self::ProjectRead => tr!("Project"),
            Self::ProjectImport => tr!("Project import"),
            Self::ProjectReopen => tr!("Import a pack"),
            Self::ArchiveScan => tr!("Archive scan"),
            Self::ArchiveTable => tr!("Archive table"),
            Self::EffectsRead => tr!("GTA VI effects"),
            Self::TextureNames => tr!("GTA VI texture names"),
            Self::NeighbourCells => tr!("GTA VI neighbouring cells"),
            Self::SetSearch => tr!("GTA VI animation sets"),
            Self::ArchetypeSearch => tr!("GTA VI archetypes"),
            Self::WorldIndex => tr!("GTA VI world index"),
            Self::AudioLinks => tr!("GTA VI audio links"),
            Self::OpusAudio => tr!("Opus audio"),
            Self::TextureCalibration => tr!("GTA VI texture calibration"),
        }
    }
}

/// A job running now, as the list draws it.
#[derive(Debug, Clone)]
pub struct RunningJob {
    pub kind: JobKind,
    /// A sentence: `Extracting models`.
    pub doing: String,
    /// How far, where the job knows its end; `None` draws a spinner.
    pub fraction: Option<f32>,
    /// Its counts and the item in hand.
    pub detail: String,
    /// Asked to stop, and still finishing what it cannot drop.
    pub stopping: bool,
}

/// What the list offers to do with what a job left.
#[derive(Debug, Clone, PartialEq)]
pub enum Then {
    Nothing,
    OpenFolder(PathBuf),
    /// `inner` under the install or folder open at `source`, revealed in the tree.
    Reveal {
        source: PathBuf,
        inner: String,
    },
    Show(DocumentId),
    Panel(crate::ui::panel_registry::PanelId),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done(Then),
    Stopped(Then),
    Failed,
}

/// A job that ended, and what it said: what it did, or why it stopped or failed.
#[derive(Debug, Clone)]
pub struct EndedJob {
    pub kind: JobKind,
    pub outcome: Outcome,
    pub said: String,
}

/// The jobs that ended, newest first.
#[derive(Default)]
pub struct JobLog {
    ended: Vec<EndedJob>,
}

impl JobLog {
    /// ⚠ One per kind: a filter typed into starts a search per keystroke, and only the last one says anything.
    pub fn record(&mut self, kind: JobKind, outcome: Outcome, said: impl Into<String>) {
        self.ended.retain(|job| job.kind != kind);
        self.ended.insert(
            0,
            EndedJob {
                kind,
                outcome,
                said: said.into(),
            },
        );
    }

    pub fn ended(&self) -> &[EndedJob] {
        &self.ended
    }

    pub fn clear(&mut self) {
        self.ended.clear();
    }
}

/// `path` as the disk has it: the part that exists in its real case, with no `..` or link in it, then the rest as given.
///
/// ⚠ `starts_with` compares spellings, so a path reached through a `..`, in another case or by a
/// short name is not under the folder a job reads until both are spelled as the disk spells them.
fn as_the_disk_has_it(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return rest.iter().rev().fold(real, |real, name| real.join(name));
        }
        let (Some(parent), Some(name)) = (at.parent(), at.file_name()) else {
            return path.to_path_buf();
        };
        rest.push(name);
        at = parent;
    }
}

impl AppState {
    /// The GTA VI archives open as what a scan finds in them.
    fn archive_scans(&self) -> impl Iterator<Item = &super::rpf9_scan::Rpf9ScanDocument> {
        self.documents.iter().filter_map(|doc| match &doc.payload {
            super::DocPayload::Rpf9(Rpf9Document::Scan(scan)) => Some(&**scan),
            _ => None,
        })
    }

    fn archive_tables(&self) -> impl Iterator<Item = &super::document::rpf9::Rpf9TableDocument> {
        self.documents.iter().filter_map(|doc| match &doc.payload {
            super::DocPayload::Rpf9(Rpf9Document::Table(table)) => Some(&**table),
            _ => None,
        })
    }
}

/// A path's last component, for a sentence that names a file.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl AppState {
    /// Every job running now, in the order the list draws them.
    pub fn running_jobs(&self) -> Vec<RunningJob> {
        [
            self.extract_jobs.job(),
            self.defrag.job(),
            self.overlay_copy.job(),
            self.search.job(),
            self.file_index.job(),
            self.keys.job(),
            self.game_pack.job(),
            self.name_index.job(),
            self.load_job(),
            self.save_job(),
            self.convert.job(),
            self.dlc_pack.job(),
            self.gltf.job(),
            self.dependencies.job(),
            self.compare.job(),
            self.project_build_job(),
            self.project_read_job(),
            self.project_import_job(),
            self.project_reopen_job(),
        ]
        .into_iter()
        .flatten()
        .chain(self.archive_scans().filter_map(|scan| scan.job()))
        .chain(self.archive_tables().filter_map(|table| table.job()))
        .chain(self.archive_scans().filter_map(|scan| scan.effects_job()))
        .chain(super::gta6_effects::install_job())
        .chain(super::gta6_shaders::job())
        .chain(self.texture_naming_jobs())
        .chain(self.neighbour_search_job())
        .chain(self.set_finding_jobs())
        .chain(self.map_search_jobs())
        .chain(self.audio_linking_jobs())
        .chain(super::gta6_dialogue::job())
        .chain(self.awc_opus_job())
        .chain(super::gta6_install_scan::job())
        .chain(super::ttpf_document::calibration::job())
        .collect()
    }

    /// Stop the running job of `kind`: at once where it can drop its work, else once it has finished what it cannot.
    pub fn cancel_job(&mut self, kind: JobKind) {
        let dropped = match kind {
            JobKind::Extraction | JobKind::Export => self.extract_jobs.request_cancel(kind),
            JobKind::Defragment => {
                self.defrag.cancel();
                false
            }
            JobKind::OverlayCopy => {
                self.overlay_copy.cancel();
                false
            }
            JobKind::Search => {
                let running = self.search.is_running();
                self.search.stop();
                running
            }
            JobKind::FileIndex => self.file_index.stop(),
            JobKind::Keys => {
                self.keys.cancel();
                false
            }
            JobKind::GamePackBake => {
                self.game_pack.cancel();
                false
            }
            JobKind::NameScan => {
                let running = self.name_index.is_scanning();
                if running {
                    self.name_index.stop_bg_scan();
                }
                running
            }
            JobKind::Load => self.cancel_loads(),
            JobKind::Save => {
                self.cancel_save();
                false
            }
            JobKind::Conversion => {
                self.convert.cancel();
                self.extract_jobs.request_cancel(kind);
                false
            }
            JobKind::DlcPack => {
                self.dlc_pack.cancel();
                false
            }
            JobKind::Gltf => self.gltf.cancel(),
            JobKind::Dependencies => self.dependencies.cancel(),
            JobKind::Compare => self.compare.cancel(),
            JobKind::ProjectBuild => {
                self.workbench.stop_building();
                self.show_info_toast(tr!(
                    "The build being written is stopped and the folder it was writing is removed. \
                     The last good build is never damaged"
                ));
                false
            }
            // ⚠ Cancelling the read of a project's files is leaving the project: it lists what the read finds.
            JobKind::ProjectRead => {
                self.leave_project_for_the_start_page();
                false
            }
            // ⚠ Stopped after the file in hand: a source is moved into place only when it is whole, so what was made stays and Undo import takes it back.
            JobKind::ProjectImport => {
                self.cancel_import();
                false
            }
            // ⚠ Stopped at the next file of the pack or the next garment: nothing is made, and the pack is never written.
            JobKind::ProjectReopen => {
                self.cancel_reopen();
                false
            }
            JobKind::ArchiveScan => {
                // ⚠ Its end is said once its worker sees the stop, by `poll_install_scan`.
                super::gta6_install_scan::stop();
                let running: Vec<_> = (self.archive_scans())
                    .filter(|scan| scan.job().is_some())
                    .collect();
                running.iter().for_each(|scan| scan.stop());
                !running.is_empty()
            }
            JobKind::ArchiveTable => {
                self.archive_tables().for_each(|table| table.stop());
                true
            }
            // ⚠ Its end is said once its worker sees the stop, by `poll_effects_reads`.
            JobKind::EffectsRead => {
                self.archive_scans().for_each(|scan| scan.stop_effects());
                super::gta6_effects::stop_install_read();
                super::gta6_shaders::stop();
                false
            }
            // ⚠ Its end is said once its worker sees the stop, by `poll_texture_naming`.
            JobKind::TextureNames => {
                self.stop_texture_naming();
                false
            }
            // ⚠ Its end is said once its worker sees the stop, by `poll_neighbour_search`.
            JobKind::NeighbourCells => {
                self.stop_neighbour_search();
                false
            }
            // ⚠ Its end is said once its worker sees the stop, by `poll_set_finding`.
            JobKind::SetSearch => {
                self.stop_set_finding();
                false
            }
            // ⚠ Its end is said once its worker sees the stop, by `poll_map_search`.
            JobKind::ArchetypeSearch => {
                self.stop_map_search();
                false
            }
            // ⚠ Its end is said once its worker sees the stop, by `poll_map_search`.
            JobKind::WorldIndex => {
                super::gta6_world::worldview::viewer().stop();
                false
            }
            // ⚠ Its end shows in the audio game data's view once its worker sees the stop.
            JobKind::AudioLinks => {
                self.stop_audio_linking();
                super::gta6_dialogue::stop();
                false
            }
            // ⚠ Its end is said once its worker sees the stop, by `poll_awc_opus`.
            JobKind::OpusAudio => {
                self.stop_awc_opus();
                false
            }
            // ⚠ Its end is said once its worker sees the stop, by `poll_texture_calibration`.
            JobKind::TextureCalibration => {
                super::ttpf_document::calibration::stop();
                false
            }
        };
        if dropped {
            self.jobs.record(
                kind,
                Outcome::Stopped(Then::Nothing),
                tr!("Stopped before it finished"),
            );
        }
    }

    /// `Err` saying `then` waits for the job reading the file at `path`, one in the folder at `path`, or the folder holding it.
    ///
    /// ⚠ A job whose worker reads a file on disk while it runs answers here,
    /// or a write replaces the file under it; a glTF's model is read before
    /// its worker starts. The Uses read does not: what it reads is dropped
    /// and read again once a file under the install is replaced
    /// (`file_replaced`), as waiting out the read would hold every save.
    pub(super) fn no_job_reads(&self, path: &Path, then: &str) -> Result<(), String> {
        self.no_job_reads_any([path], then)
    }

    /// As [`no_job_reads`](Self::no_job_reads), for each of `paths`, so the files the jobs read are looked up once.
    pub(super) fn no_job_reads_any<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a Path>,
        then: &str,
    ) -> Result<(), String> {
        self.no_job_touches(self.extract_jobs.reads(), paths, then)
    }

    /// As [`no_job_reads`](Self::no_job_reads), for what removes or relocates `path`: an extraction still opening its rows also holds every file under the rows it names.
    ///
    /// ⚠ A save is not that: it replaces one file, so it asks only about the
    /// archives an opening extraction has asked for, and a save of any other
    /// file under the install goes ahead.
    pub(super) fn no_job_holds(&self, path: &Path, then: &str) -> Result<(), String> {
        self.no_job_touches(self.extract_jobs.holds(), [path], then)
    }

    fn no_job_touches<'a>(
        &self,
        extraction: Option<(String, &[PathBuf])>,
        paths: impl IntoIterator<Item = &'a Path>,
        then: &str,
    ) -> Result<(), String> {
        let reads = [
            extraction,
            self.convert.reads(),
            self.dlc_pack.reads(),
            self.compare.reads(),
        ];
        let reading: Vec<(String, Vec<PathBuf>)> = (reads.into_iter().flatten())
            .map(|(job, files)| {
                (
                    job,
                    files.iter().map(|file| as_the_disk_has_it(file)).collect(),
                )
            })
            .collect();
        if reading.is_empty() {
            return Ok(());
        }
        for path in paths {
            let has_it = as_the_disk_has_it(path);
            let touched = reading.iter().find(|(_, files)| {
                (files.iter()).any(|file| file.starts_with(&has_it) || has_it.starts_with(file))
            });
            if let Some((job, _)) = touched {
                return Err(tr!(
                    "`{path}` is being read by {job}; {then} once that is done",
                    job = job,
                    then = then,
                    path = path.display()
                ));
            }
        }
        Ok(())
    }

    /// The per-frame tick of the jobs whose panels may not be drawn.
    pub fn poll_jobs(&mut self) {
        self.archive_tables().for_each(|table| table.tree().poll());
        self.poll_defrag();
        self.poll_overlay_copy();
        self.poll_overlay_refresh();
        self.poll_search();
        self.tick_file_index();
        self.poll_name_scan();
        self.poll_conversion();
        self.poll_dlc_pack();
        self.poll_gltf();
        self.poll_dependencies();
        self.poll_compare();
        self.poll_workbench();
        self.poll_effects_reads();
        self.poll_texture_naming();
        self.poll_neighbour_search();
        self.poll_set_finding();
        self.poll_map_search();
        self.poll_awc_opus();
        self.poll_install_scan();
        self.poll_texture_calibration();
        self.poll_table_opens();
        self.poll_archive_saves();
    }

    fn poll_search(&mut self) {
        if !self.search.poll() {
            return;
        }
        let progress = &self.search.progress;
        let said = tr!(
            "{hits} hit(s) in {files_seen} file(s) for {asked}",
            hits = progress.hits,
            files_seen = progress.files_seen,
            asked = self.search.asked()
        );
        let then = Then::Panel(crate::ui::panels::registry::ids::SEARCH);
        self.jobs.record(JobKind::Search, Outcome::Done(then), said);
    }

    fn poll_name_scan(&mut self) {
        if let Some(names) = self.name_index.drain_bg() {
            let said = tr!(
                "{files} ready for the pickers",
                files = crate::state::front_door::counted(names, "game name")
            );
            self.jobs
                .record(JobKind::NameScan, Outcome::Done(Then::Nothing), said);
        }
    }
}
