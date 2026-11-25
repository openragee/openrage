//! Every background job is in one list the status bar opens, with its progress and a Cancel,
//! and nothing veils the window while one runs.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::rpf::crypto::NoOpCryptoProvider;
use or_job::{Cancel, Step};

use super::jobs::{JobKind, Outcome};
use super::{AppState, ExtractionResult, SaveResult};

const ALL: [JobKind; 16] = [
    JobKind::Extraction,
    JobKind::Defragment,
    JobKind::OverlayCopy,
    JobKind::Search,
    JobKind::FileIndex,
    JobKind::Keys,
    JobKind::GamePackBake,
    JobKind::NameScan,
    JobKind::Load,
    JobKind::Save,
    JobKind::Conversion,
    JobKind::DlcPack,
    JobKind::Gltf,
    JobKind::Dependencies,
    JobKind::Compare,
    JobKind::TextureNames,
];

const DEADLINE: Duration = Duration::from_secs(5);

/// A stand-in's work: nothing until it is stopped.
fn until_stopped(cancel: &Cancel) {
    let until = Instant::now() + DEADLINE;
    while !cancel.is_set() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A process that does nothing until it is killed, as the helper does while it bakes.
fn sleeper() -> std::process::Command {
    #[cfg(unix)]
    {
        let mut command = std::process::Command::new("sleep");
        command.arg("60");
        command
    }
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("ping");
        command.args(["-n", "5", "127.0.0.1"]);
        command
    }
}

/// One of each kind running, with a stand-in wherever a real run needs an install or the network.
fn every_kind_running(scratch: &Path) -> AppState {
    let mut state = AppState::default();
    state.documents.clear();
    let crypto = Arc::new(NoOpCryptoProvider::new());
    std::fs::write(scratch.join("a.meta"), b"WEAPON_RAILGUN").expect("a file to search");
    std::fs::write(scratch.join("notes.txt"), b"a file to open").expect("a file to open");

    state.extract_jobs.start(
        JobKind::Extraction,
        "models".into(),
        scratch.join("out"),
        Vec::new(),
        |cancel, progress| {
            or_job::run(0..1_000_000u32, &cancel, &progress, |_| {
                std::thread::sleep(Duration::from_millis(1));
                Step::Done(0)
            })
        },
    );
    state.defrag.start(
        scratch.join("x64a.rpf"),
        "Defragmenting x64a.rpf".into(),
        |progress, cancel| {
            progress.reached(1, 4);
            until_stopped(cancel);
            super::defrag_job::Outcome::Stopped
        },
    );
    state.overlay_copy.start(
        super::overlay_copy::OverlayCopyOffer {
            source: scratch.join("update.rpf"),
            dest: scratch.join("mods").join("update.rpf"),
            bytes: 2048,
            replaces: None,
        },
        |cancel, progress| {
            progress.reached(1024, 2048);
            until_stopped(cancel);
            Err("stopped".into())
        },
    );
    state.search.start(
        super::search_session::SearchRequest {
            pattern: "railgun".into(),
            mode: super::search_session::SearchMode::Text,
            case_sensitive: false,
            extensions: Vec::new(),
            under: String::new(),
            max_hits: 0,
        },
        scratch.to_path_buf(),
        crypto.clone(),
    );
    state.file_index.walk_with(scratch.to_path_buf(), |stop| {
        let until = Instant::now() + DEADLINE;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    state.keys.spawn(
        "Reading the keys from the game's executable",
        "Starting",
        0,
        |_, cancel| {
            until_stopped(cancel);
            ExtractionResult::Stopped
        },
    );
    state
        .game_pack
        .begin(sleeper())
        .expect("a process that waits can be run");
    state.name_index.start_scan(
        scratch.to_path_buf(),
        crypto,
        or_title::MountOptions::install_only(),
    );
    state.load_file(scratch.join("notes.txt"));
    state.start_save(scratch.join("saved.ypt"), |cancel| {
        until_stopped(cancel);
        SaveResult::Stopped
    });
    state
        .convert
        .start("prop.ydr".into(), false, false, Vec::new(), |cancel, _| {
            until_stopped(&cancel);
            None
        });
    state
        .dlc_pack
        .start("mypack".into(), scratch.to_path_buf(), |cancel, _| {
            until_stopped(&cancel);
            Err("stopped".into())
        });
    state
        .gltf
        .start("Exporting prop.ydr to glTF".into(), |cancel| {
            until_stopped(cancel);
            None
        });
    state.dependencies.start(|cancel, _| {
        until_stopped(cancel);
        Err("stopped".into())
    });
    // A comparison reads on unheard once dropped, as an open does.
    state
        .compare
        .start("a.ymt ↔ b.ymt".into(), Vec::new(), |_| {
            std::thread::sleep(DEADLINE);
            super::compare::Ended::Failed("stopped".into())
        });
    (state.texture_naming).push(super::gta6_texture_naming::TextureNaming::run(
        "Naming".into(),
        |cancel, _| {
            until_stopped(cancel);
            super::gta6_texture_naming::NamingEnded::Stopped
        },
    ));
    state
}

/// Draws the status bar, returning each labelled widget and its centre.
pub(super) fn frame(
    ctx: &egui::Context,
    state: &mut AppState,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Pos2)> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| crate::ui::panels::status_bar::draw(ui, state));
    output
        .platform_output
        .accesskit_update
        .map(|update| {
            update
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    let b = node.bounds()?;
                    let centre =
                        egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
                    let text = node.label().or_else(|| node.value())?;
                    Some((text.to_string(), centre))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn click(ctx: &egui::Context, state: &mut AppState, naming: &str) {
    let shown = frame(ctx, state, Vec::new());
    let at = shown
        .iter()
        .find(|(label, _)| label == naming)
        .unwrap_or_else(|| panic!("nothing on screen is labelled `{naming}`: {shown:?}"))
        .1;
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(ctx, state, vec![egui::Event::PointerMoved(at), press(true)]);
    frame(ctx, state, vec![press(false)]);
}

fn poll_everything(state: &mut AppState) {
    state.poll_jobs();
    state.poll_extract_job();
    state.poll_key_extraction();
    state.poll_game_pack();
    state.poll_load_result();
    state.poll_save_result();
}

#[test]
fn every_kind_of_job_is_listed_while_it_runs_and_stops_on_a_cancel_from_the_list() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut state = every_kind_running(scratch.path());
    let running: Vec<JobKind> = state.running_jobs().iter().map(|job| job.kind).collect();
    for kind in ALL {
        assert!(
            running.contains(&kind),
            "{kind:?} is not listed: {running:?}"
        );
    }

    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    click(&ctx, &mut state, &format!("{} jobs running", ALL.len()));
    let listed: Vec<String> = frame(&ctx, &mut state, Vec::new())
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    for job in state.running_jobs() {
        assert!(
            listed.iter().any(|label| label == &job.doing),
            "the list does not say what the {:?} job is doing, `{}`: {listed:?}",
            job.kind,
            job.doing
        );
    }

    for kind in ALL {
        click(&ctx, &mut state, &format!("Cancel {}", kind.label()));
    }
    let after: Vec<String> = frame(&ctx, &mut state, Vec::new())
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    for job in state.running_jobs() {
        assert!(
            job.stopping,
            "{:?} was cancelled and says nothing of it",
            job.kind
        );
    }
    assert!(
        after.iter().any(|label| label == "Stopping…"),
        "a job that cannot stop at once does not say it is stopping: {after:?}"
    );

    let until = Instant::now() + DEADLINE;
    while !state.running_jobs().is_empty() && Instant::now() < until {
        poll_everything(&mut state);
        std::thread::sleep(Duration::from_millis(5));
    }
    // A worker that ended after the last poll is no longer listed, and says so at the next poll.
    poll_everything(&mut state);
    let still: Vec<JobKind> = state.running_jobs().iter().map(|job| job.kind).collect();
    assert!(still.is_empty(), "still running after a Cancel: {still:?}");
    for kind in ALL {
        let ended = state.jobs.ended().iter().find(|job| job.kind == kind);
        assert!(
            matches!(ended.map(|job| &job.outcome), Some(Outcome::Stopped(_))),
            "{kind:?} did not end as stopped: {:?}",
            state.jobs.ended()
        );
    }
}

/// The bake was a window over a dimmed screen that said to keep working.
#[test]
fn the_bake_runs_as_a_job_and_nothing_veils_the_window() {
    let mut state = AppState::default();
    state
        .game_pack
        .begin(sleeper())
        .expect("a process that waits can be run");
    let ctx = egui::Context::default();
    let centre = egui::pos2(800.0, 450.0);
    for _ in 0..3 {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 900.0),
            )),
            events: vec![egui::Event::PointerMoved(centre)],
            ..Default::default()
        });
        crate::ui::draw(&ctx, &mut state, None);
        let _ = ctx.end_pass();
    }
    let over = ctx.layer_id_at(centre).map(|layer| layer.order);
    assert!(
        !matches!(
            over,
            Some(egui::Order::Foreground | egui::Order::Tooltip | egui::Order::Debug)
        ),
        "something is drawn over the middle of the window while the bake runs: {over:?}"
    );
    assert!(
        state
            .running_jobs()
            .iter()
            .any(|job| job.kind == JobKind::GamePackBake),
        "the bake is not in the list"
    );
    state.cancel_job(JobKind::GamePackBake);
    let until = Instant::now() + DEADLINE;
    while state.game_pack.is_baking() && Instant::now() < until {
        state.poll_game_pack();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!state.game_pack.is_baking(), "the bake did not stop");
}

/// Where a kind of job is held to be listed and stopped from the list: by `every_kind_of_job...` above, or by the test named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Here,
    By(&'static str),
}

/// ⚠ No wildcard: a kind added to `JobKind` fails to compile here until it is placed, and
/// `EVERY` must then list it for the test below to pass.
fn held(kind: JobKind) -> Held {
    match kind {
        JobKind::Extraction
        | JobKind::Defragment
        | JobKind::OverlayCopy
        | JobKind::Search
        | JobKind::FileIndex
        | JobKind::Keys
        | JobKind::GamePackBake
        | JobKind::NameScan
        | JobKind::Load
        | JobKind::Save
        | JobKind::Conversion
        | JobKind::DlcPack
        | JobKind::Gltf
        | JobKind::Dependencies
        | JobKind::Compare
        | JobKind::TextureNames => Held::Here,
        JobKind::Export => {
            Held::By("an_export_as_write_is_listed_as_an_export_and_cancel_export_reaches_it")
        }
        JobKind::ProjectBuild => {
            Held::By("a_build_stopped_from_the_job_list_removes_what_it_was_writing_and_says_so")
        }
        JobKind::ProjectRead => {
            Held::By("reading_a_project_is_a_job_in_the_status_bar_until_its_files_are_read")
        }
        JobKind::ProjectImport => Held::By(
            "an_import_cancelled_the_moment_it_starts_loses_no_file_whatever_it_had_reached",
        ),
        JobKind::ProjectReopen => Held::By("stop_in_the_job_list_asks_the_worker_to_stop"),
        JobKind::ArchiveScan => {
            Held::By("a_scan_runs_off_the_ui_thread_in_the_job_list_and_stops_when_asked")
        }
        JobKind::ArchiveTable => Held::By("a_child_table_is_a_job_that_can_be_cancelled"),
        JobKind::EffectsRead => Held::By(
            "an_effect_container_is_read_as_a_job_with_progress_that_stops_when_asked_and_says_how_it_ended",
        ),
        JobKind::NeighbourCells => Held::By(
            "the_cells_across_a_gta_vi_cell_s_borders_are_found_in_its_scan_as_a_job_and_open",
        ),
        JobKind::SetSearch => {
            Held::By("a_set_another_body_stores_is_found_among_the_animation_bodies_of_the_scan")
        }
        JobKind::ArchetypeSearch => {
            Held::By("an_archetype_a_map_names_is_found_among_the_bodies_of_its_scan_as_a_job")
        }
        JobKind::WorldIndex => Held::By(
            "the_world_index_is_a_job_of_its_own_and_a_folder_with_no_game_fails_naming_it",
        ),
        JobKind::AudioLinks => Held::By(
            "what_gta_vi_audio_game_data_links_to_is_found_in_its_scan_as_a_job_and_opens",
        ),
        JobKind::OpusAudio => {
            Held::By("an_opus_stream_plays_exports_and_is_replaced_on_a_worker_the_list_stops")
        }
        JobKind::TextureCalibration => {
            Held::By("a_calibration_of_texture_decoding_is_a_job_that_stops_when_asked")
        }
    }
}

/// Every kind of job there is.
const EVERY: [JobKind; 31] = [
    JobKind::Extraction,
    JobKind::Defragment,
    JobKind::OverlayCopy,
    JobKind::Search,
    JobKind::FileIndex,
    JobKind::Keys,
    JobKind::GamePackBake,
    JobKind::NameScan,
    JobKind::Load,
    JobKind::Save,
    JobKind::Conversion,
    JobKind::Export,
    JobKind::DlcPack,
    JobKind::Gltf,
    JobKind::Dependencies,
    JobKind::Compare,
    JobKind::ProjectBuild,
    JobKind::ProjectRead,
    JobKind::ProjectImport,
    JobKind::ProjectReopen,
    JobKind::ArchiveScan,
    JobKind::ArchiveTable,
    JobKind::EffectsRead,
    JobKind::TextureNames,
    JobKind::NeighbourCells,
    JobKind::SetSearch,
    JobKind::ArchetypeSearch,
    JobKind::WorldIndex,
    JobKind::AudioLinks,
    JobKind::OpusAudio,
    JobKind::TextureCalibration,
];

#[test]
fn the_kinds_the_list_test_runs_are_those_no_other_test_holds() {
    for kind in ALL {
        assert_eq!(
            held(kind),
            Held::Here,
            "{kind:?} is run here and held elsewhere"
        );
    }
    let here: Vec<JobKind> = EVERY
        .into_iter()
        .filter(|&kind| held(kind) == Held::Here)
        .collect();
    assert_eq!(
        here, ALL,
        "a kind held here is left out of `ALL`, or one run in `ALL` is held elsewhere"
    );
    let named: Vec<&str> = EVERY
        .into_iter()
        .filter_map(|kind| match held(kind) {
            Held::By(test) => Some(test),
            Held::Here => None,
        })
        .collect();
    let sources = [
        include_str!("what_converting_still_gets_wrong_after_r55c.rs"),
        include_str!("building_targets_from_the_app.rs"),
        include_str!("the_front_door_opens_and_closes.rs"),
        include_str!("dropping_and_importing_folders.rs"),
        include_str!("reopening_a_built_pack_from_the_app.rs"),
        include_str!("one_place_for_background_jobs.rs"),
        include_str!("opening_an_rpf9_archive.rs"),
        include_str!("document/rpf9_tests.rs"),
        include_str!("opening_a_gta_vi_body.rs"),
        include_str!("gta6_set_finding_tests.rs"),
        include_str!("gta6_map_tests.rs"),
        include_str!("gta6_audio_links_tests.rs"),
        include_str!("opus_on_a_worker.rs"),
        include_str!("gta6_bcpack_calibration.rs"),
    ];
    for test in named {
        let defined = sources
            .iter()
            .any(|text| text.contains(&format!("fn {test}")));
        assert!(defined, "no test is called `{test}`");
    }
}
