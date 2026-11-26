//! A build declared in the app can write a singleplayer DLC or an OIV package, and move what it ships
//! to an edition, and the project file remembers which.
//!
//! ⚠ Built with `AppState::from_prefs` and a project in a scratch folder, as the Project workspace's
//! own cases are: no install, no keys, and a worker for each scan and build a case waits on.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::decl::File;
use or_io::gta5::gen8::GrcTexture;
use or_io::gta5::PgDictionary;
use or_project::{Container, Output, OutputKind, Project, Target};

use super::workbench::{BuildDot, LastBuild};
use super::{AppState, Prefs};

fn write(path: &Path, body: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A Legacy texture dictionary of one block-compressed texture.
fn legacy_ytd() -> Vec<u8> {
    let format = or_io::codec::texture::GrcTextureFormat::DXT1;
    let texture = GrcTexture {
        name: "road".into(),
        width: 16,
        height: 16,
        depth: 1,
        mip_count: 1,
        mip_stride: u16::try_from(format.legacy_mip_stride(16)).unwrap(),
        format: format.to_raw_format(),
        backing_store: vec![0x5a; format.total_size(16, 16, 1) as usize],
        ..Default::default()
    };
    PgDictionary {
        codes: vec![or_io::string_hash(b"road")],
        entries: vec![Some(Box::new(texture))],
        ..Default::default()
    }
    .to_bytes()
    .unwrap()
}

/// A project holding the files `files`, with the builds `outputs`, opened in a state.
fn opened(dir: &Path, files: &[(&str, Vec<u8>)], outputs: Vec<Output>) -> (AppState, PathBuf) {
    let folder = dir.join("police_pack");
    let members: Vec<PathBuf> = files
        .iter()
        .map(|(rel, body)| {
            write(&folder.join(rel), body);
            folder.join(rel)
        })
        .collect();
    let container = Container {
        outputs,
        ..Container::default()
    };
    let file = folder.join("police_pack.openrage-proj");
    Project::create(&file, &container, &members).expect("a new project");
    let mut state = AppState::from_prefs(Prefs::default());
    state.documents.clear();
    state.active_document = None;
    state.load_project_from(file.clone());
    (state, file)
}

fn until(state: &mut AppState, what: &str, done: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        state.poll_workbench();
        if done(state) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_dlc_is_declared_for_an_edition_remembered_by_the_file_and_built_with_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, file) = opened(
        dir.path(),
        &[
            ("data/props.ityp", vec![1, 2, 3]),
            ("x64/models/cdimages/peds.rpf/road.ytd", legacy_ytd()),
        ],
        Vec::new(),
    );

    let id = state
        .add_build(
            "police_dlc",
            OutputKind::SingleplayerDlc,
            Some(Target::Enhanced),
        )
        .expect("a pack name is a build");
    state.save_project_to(file.clone()).expect("saved");
    let saved = Project::load(&file).expect("it reads");
    let output = saved.container.output(&id).expect("the build is kept");
    assert_eq!(output.kind, OutputKind::SingleplayerDlc);
    assert_eq!(output.target, Some(Target::Enhanced));

    until(&mut state, "the scan", |s| s.workbench.is_scanned());
    until(&mut state, "the plan", |s| !s.workbench.is_blocked(&id));
    state.build_selected();
    until(&mut state, "the build", |s| {
        !s.workbench.builds().is_running()
    });

    let built = state.workbench.builds().last(&id).expect("it ended");
    let LastBuild::Built(done) = built else {
        panic!("the build stopped: {built:?}");
    };
    let part = &done.parts[0];
    assert_eq!(part.dlclist_entry.as_deref(), Some("dlcpacks:/police_dlc/"));
    assert!(part.folder.join("dlc.rpf").is_file());
    assert!(
        part.reloaded
            .iter()
            .any(|(path, reader)| path.ends_with("road.ytd") && reader.contains("Enhanced")),
        "the dictionary inside the nested archive reloads in the edition: {:?}",
        part.reloaded
    );
    assert_eq!(state.workbench.dot(&id), BuildDot::Built);
}

#[test]
fn a_pack_built_by_the_app_is_read_against_the_line_about_a_story_pack_for_its_edition() {
    use or_project::acceptance::{Place, Seen};

    let dir = tempfile::tempdir().unwrap();
    let files = [("x64/models/cdimages/peds.rpf/road.ytd", legacy_ytd())];
    let (mut state, _) = opened(dir.path(), &files, Vec::new());
    let id = state
        .add_build(
            "police_dlc",
            OutputKind::SingleplayerDlc,
            Some(Target::Enhanced),
        )
        .expect("a pack name is a build");
    until(&mut state, "the plan", |s| {
        s.workbench.is_scanned() && !s.workbench.is_blocked(&id)
    });
    state.build_selected();
    until(&mut state, "the build", |s| {
        !s.workbench.builds().is_running()
    });

    let built = state.workbench.builds().last(&id).expect("it ended");
    let LastBuild::Built(done) = built else {
        panic!("the build stopped: {built:?}");
    };
    let part = &done.parts[0];
    // The worker read the pack it wrote: the archive is a kind of its own, beside the kinds of the files in it.
    assert!(
        part.kinds.contains("rpf") && part.kinds.contains("ytd"),
        "{:?}",
        part.kinds
    );
    assert_eq!(part.target, Some(Target::Enhanced));
    // ⚠ The test's own copy of the sheet with no result, so that what a person writes into the sheet the tree ships changes nothing here.
    let mut own: serde_json::Value = serde_json::from_str(or_project::acceptance::SHEET).unwrap();
    for line in own["lines"].as_array_mut().unwrap() {
        line["results"] = serde_json::json!([]);
    }
    let sheet = or_project::acceptance::Sheet::parse(&own.to_string()).unwrap();
    assert_eq!(
        sheet.seen(&part.kinds, part.target, Place::StoryMode),
        [(Target::Enhanced, Seen::Not(vec!["rpf".to_string()]))],
        "the line about a story pack is the one line about what this build wrote"
    );
}

#[test]
fn a_pack_whose_files_have_no_type_is_blocked_with_the_file_named() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = opened(
        dir.path(),
        &[("data/vehicles.meta", b"<x/>".to_vec())],
        Vec::new(),
    );
    let id = state
        .add_build("police_dlc", OutputKind::SingleplayerDlc, None)
        .expect("a build");

    until(&mut state, "the plan to be blocked", |s| {
        s.workbench.is_scanned() && s.workbench.is_blocked(&id)
    });

    let said: Vec<String> = state
        .workbench
        .resources_of(&id)
        .iter()
        .flat_map(|r| r.plan.problems.clone())
        .collect();
    assert!(
        said.iter()
            .any(|p| p.contains("data/vehicles.meta") && p.contains("File types")),
        "{said:?}"
    );
}

#[test]
fn a_package_is_built_by_the_app_into_a_folder_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, file) = opened(
        dir.path(),
        &[("readme.txt", b"read me".to_vec())],
        Vec::new(),
    );
    let id = state
        .add_build("police_oiv", OutputKind::OivPackage, None)
        .expect("a build");

    until(&mut state, "the plan", |s| {
        s.workbench.is_scanned() && !s.workbench.is_blocked(&id)
    });
    state.build_selected();
    until(&mut state, "the build", |s| {
        !s.workbench.builds().is_running()
    });

    let built = state.workbench.builds().last(&id).expect("it ended");
    assert!(matches!(built, LastBuild::Built(_)), "{built:?}");
    let package = file
        .parent()
        .unwrap()
        .join("build/police_oiv/police_oiv.oiv");
    assert!(package.is_file(), "{} was not written", package.display());
}

#[test]
fn a_build_stopped_from_the_job_list_removes_what_it_was_writing_and_says_so() {
    use crate::state::jobs::JobKind;

    let dir = tempfile::tempdir().unwrap();
    let (mut state, file) = opened(
        dir.path(),
        &[("data/a.txt", b"a".to_vec()), ("data/b.txt", b"b".to_vec())],
        Vec::new(),
    );
    let id = state
        .add_build("police_pack", OutputKind::FivemResource, None)
        .expect("a build");
    until(&mut state, "the plan", |s| {
        s.workbench.is_scanned() && !s.workbench.is_blocked(&id)
    });

    state.build_selected();
    // ⚠ The worker plans the project from the disk before it writes a file, so the flag is set well before the first file.
    state.cancel_job(JobKind::ProjectBuild);
    assert!(state.workbench.builds().is_stopping());
    until(&mut state, "the build to end", |s| {
        !s.workbench.builds().is_running()
    });

    let ended = state.workbench.builds().last(&id).expect("it ended");
    let LastBuild::Failed(reasons) = ended else {
        panic!("a stopped build is not a built one: {ended:?}");
    };
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("Stopped") && r.contains("last good build")),
        "{reasons:?}"
    );
    assert_eq!(state.workbench.dot(&id), BuildDot::Failed);
    let live = file.parent().unwrap().join("build/police_pack");
    assert!(!live.exists(), "nothing was swapped in: {}", live.display());
}

/// A clothing project `tshirts` with a jacket and a hat dropped on it, opened in a state.
fn a_clothing_project(dir: &Path) -> (AppState, PathBuf) {
    use crate::state::front_door::{NewProject, Template};

    let mut draft = NewProject::new(Template::Clothing, dir.to_path_buf());
    draft.set_name("tshirts".into());
    let file = draft.create().expect("a clothing project").file;
    let folder = file.parent().unwrap().to_path_buf();
    let mut dropped = Vec::new();
    for (name, body) in [
        (
            "mp_m_freemode_01_mp_m_t^jbib_000_u.ydd",
            or_source::sample::drawables_of("jacket"),
        ),
        (
            "mp_m_freemode_01_mp_m_t^jbib_diff_000_a_uni.ytd",
            or_source::sample::textures_of("jacket texture"),
        ),
        (
            "mp_m_freemode_01_p_mp_m_t^p_head_000.ydd",
            or_source::sample::drawables_of("hat"),
        ),
        (
            "mp_m_freemode_01_p_mp_m_t^p_head_diff_000_a.ytd",
            or_source::sample::textures_of("hat texture"),
        ),
    ] {
        let path = dir.join("drop").join(name);
        write(&path, &body);
        dropped.push(path);
    }
    or_wardrobe::edit::add_clothes(&folder.join("clothing"), &dropped).expect("the garments");
    let mut state = AppState::from_prefs(Prefs::default());
    state.documents.clear();
    state.active_document = None;
    state.load_project_from(file.clone());
    (state, file)
}

#[test]
fn a_clothing_project_builds_a_singleplayer_dlc_and_the_clothing_check_reads_it_back_through_the_archives(
) {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = a_clothing_project(dir.path());
    let id = state
        .add_build("tshirts_dlc", OutputKind::SingleplayerDlc, None)
        .expect("a build");
    until(&mut state, "the plan", |s| {
        s.workbench.is_scanned() && !s.workbench.resources_of(&id).is_empty()
    });
    assert!(
        !state.workbench.is_blocked(&id),
        "a clothing pack ships in a pack: {:?}",
        state
            .workbench
            .resources_of(&id)
            .iter()
            .flat_map(|r| r.plan.problems.clone())
            .collect::<Vec<_>>()
    );
    state.workbench.selected_output = Some(id.clone());
    state.build_selected();
    until(&mut state, "the build", |s| {
        !s.workbench.builds().is_running()
    });

    let built = state.workbench.builds().last(&id).expect("it ended");
    let LastBuild::Built(done) = built else {
        panic!("the build did not finish: {built:?}");
    };
    assert!(
        done.pack_problems().is_empty(),
        "{:?}",
        done.pack_problems()
    );
    let checks = &done.parts[0].pack_checks;
    assert_eq!(checks.len(), 1, "the clothing content is checked");
    assert!(
        checks[0]
            .1
            .iter()
            .any(|c| c.name.contains("variation metadata") && c.passed),
        "the variation metadata is read out of the archive: {:?}",
        checks[0].1
    );
    assert_eq!(state.workbench.dot(&id), BuildDot::Built);
}

#[test]
fn a_build_that_went_through_freezes_the_drawables_and_says_so_and_a_rebuild_that_would_move_one_is_blocked(
) {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, file) = a_clothing_project(dir.path());
    let folder = file.parent().unwrap().join("clothing");
    let id = state
        .add_build("tshirts_resource", OutputKind::FivemResource, None)
        .expect("a build");
    until(&mut state, "the plan", |s| {
        s.workbench.is_scanned() && !s.workbench.resources_of(&id).is_empty()
    });
    assert!(!folder.join("wardrobe.lock.json").exists());
    state.workbench.selected_output = Some(id.clone());
    state.build_selected();
    until(&mut state, "the build", |s| {
        !s.workbench.builds().is_running()
    });

    let LastBuild::Built(done) = state.workbench.builds().last(&id).expect("it ended") else {
        panic!("the build did not finish");
    };
    assert_eq!(done.frozen.len(), 1, "{:?}", done.frozen);
    assert!(
        done.frozen[0].contains("The places of the 2 garments"),
        "{:?}",
        done.frozen
    );
    assert!(folder.join("wardrobe.lock.json").exists());

    // A second jacket goes in front of the hat's slot-mate nothing, and the first leaves: only a move is refused.
    let mut w = or_wardrobe::Wardrobe::read(&folder).unwrap();
    let mut second = w.garments[0].clone();
    second.id = or_wardrobe::model::new_id();
    w.garments.insert(0, second);
    w.write(&folder).unwrap();
    until(&mut state, "the plan again", |s| {
        s.workbench.is_scanned() && s.workbench.is_blocked(&id)
    });
    assert!(
        state
            .workbench
            .resources_of(&id)
            .iter()
            .flat_map(|r| r.plan.problems.clone())
            .any(|p| p.contains("was jbib 000")),
        "the plan names the garment that would move"
    );
}
