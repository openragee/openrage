//! What a launch opens: the remembered install with no click, else the start
//! page and its one click; switching the game; and what a new session and an
//! opened project keep mounted.
//!
//! ⚠ Built with `AppState::from_prefs`, not `new`: `new` reads `prefs.json`, and a remembered
//! install is exactly what these cases vary.

use std::path::Path;
use std::time::{Duration, Instant};

use super::document::{Document, TextDocument};
use super::front_door::Switch;
use super::project::{load_project, save_project, ProjectDocument, ProjectFile};
use super::rpf_session::RpfEntryBinding;
use super::{AppState, CloseAnswer, DocPayload, DocumentKind, Game, LoadingState, Prefs};
use crate::platform::new_window::fake;
use crate::ui::dock::{Workspace, WorkspaceDocks};

fn remembering(game: Game, root: &Path) -> Prefs {
    let mut prefs = Prefs::default();
    prefs.game_paths.set(game, Some(root.to_path_buf()));
    prefs.active_game = Some(game);
    prefs
}

fn install(state: &AppState) -> Option<&Document> {
    state
        .documents
        .iter()
        .find(|d| d.kind == DocumentKind::GameRoot)
}

/// Every labelled widget one frame of `draw` lays out on a 1600×900 screen, and its centre.
fn labelled(
    ctx: &egui::Context,
    state: &mut AppState,
    events: Vec<egui::Event>,
    draw: fn(&mut egui::Ui, &mut AppState),
) -> Vec<(String, egui::Pos2)> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| draw(ui, state));
    output
        .platform_output
        .accesskit_update
        .map(|update| {
            update
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    let text = node.label().or_else(|| node.value())?;
                    let b = node.bounds()?;
                    let centre =
                        egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
                    Some((text.to_string(), centre))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn document_panel(ui: &mut egui::Ui, state: &mut AppState) {
    crate::ui::panels::document_view::draw(ui, state, None);
}

fn click(ctx: &egui::Context, state: &mut AppState, at: egui::Pos2) {
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    labelled(
        ctx,
        state,
        vec![egui::Event::PointerMoved(at), press(true)],
        document_panel,
    );
    labelled(ctx, state, vec![press(false)], document_panel);
}

#[test]
fn a_restart_opens_the_remembered_install_in_browse_with_its_tree_open() {
    let dir = tempfile::tempdir().expect("an install folder");
    std::fs::write(dir.path().join("commandline.txt"), b"-windowed").expect("a file in it");
    let mut prefs = remembering(Game::GtaVLegacy, dir.path());
    prefs.dock_layouts =
        crate::ui::dock_layout::capture(&WorkspaceDocks::default(), Workspace::Model);

    let mut state = AppState::from_prefs(prefs);
    let (id, root) = match install(&state) {
        Some(doc) => match &doc.payload {
            DocPayload::Rpf(rpf) => (doc.id, rpf.session.root_id()),
            _ => panic!("an install mounts as an archive tree"),
        },
        None => panic!("the remembered install is not open"),
    };
    assert_eq!(state.active_document, Some(id), "the install is not active");
    assert_eq!(
        state.active_workspace,
        Workspace::Browse,
        "the install opens in Browse, whatever workspace was last on screen"
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while state.expander.is_in_flight(id, root) {
        assert!(
            Instant::now() < deadline,
            "the install's folder was never read"
        );
        state.poll_expansions();
        std::thread::sleep(Duration::from_millis(5));
    }
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let rows = labelled(
        &ctx,
        &mut state,
        Vec::new(),
        crate::ui::panels::hierarchy::draw,
    );
    assert!(
        rows.iter()
            .any(|(text, _)| text.contains("commandline.txt")),
        "the Files view does not show the install's files with no click: {rows:?}"
    );
}

#[test]
fn a_launch_with_no_install_to_open_starts_on_the_start_page() {
    let gone = tempfile::tempdir().expect("a scratch folder");
    let launches = [
        ("a first run", Prefs::default()),
        (
            "a remembered install that is gone",
            remembering(Game::GtaVLegacy, &gone.path().join("moved")),
        ),
    ];
    for (launch, prefs) in launches {
        let mut state = AppState::from_prefs(prefs);
        assert!(state.documents.is_empty(), "{launch} opened something");
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let shown: Vec<String> = labelled(&ctx, &mut state, Vec::new(), document_panel)
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        for offer in ["Open an install", "Open file…", "Open folder…"] {
            assert!(
                shown.iter().any(|text| text.contains(offer)),
                "{launch}: the start page does not offer `{offer}`: {shown:?}"
            );
        }
    }
}

#[test]
fn a_console_build_has_a_row_only_once_its_folder_is_known() {
    let folder = tempfile::tempdir().expect("a build's folder");
    let mut known = Prefs::default();
    known
        .game_paths
        .set(Game::GtaVI, Some(folder.path().to_path_buf()));
    let said = |prefs: Prefs| -> Vec<String> {
        let mut state = AppState::from_prefs(prefs);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        labelled(&ctx, &mut state, Vec::new(), document_panel)
            .into_iter()
            .map(|(text, _)| text)
            .collect()
    };
    let platform = or_title::game::Platform::XboxSeries.label();
    let shown = said(known);
    assert!(
        shown.iter().any(|text| text.starts_with(platform)),
        "the row of a known console build does not name its platform: {shown:?}"
    );
    let shown = said(Prefs::default());
    assert!(
        !shown.iter().any(|text| text.contains(Game::GtaVI.label())),
        "a console build nobody added has a row: {shown:?}"
    );
    assert!(
        shown.iter().any(|text| text.contains("Add a game folder…")),
        "the start page does not offer to add one: {shown:?}"
    );
}

#[test]
fn a_first_run_reaches_the_install_it_found_in_one_click() {
    let dir = tempfile::tempdir().expect("an install folder");
    let found = dir.path().to_path_buf();
    let mut prefs = Prefs::default();
    prefs.game_paths.set(Game::GtaVLegacy, Some(found.clone()));
    let mut state = AppState::from_prefs(prefs);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();

    let open = labelled(&ctx, &mut state, Vec::new(), document_panel)
        .into_iter()
        .find(|(text, _)| text == "Open")
        .map(|(_, at)| at)
        .expect("the start page offers no Open for the install discovery found");
    click(&ctx, &mut state, open);
    // ⚠ Gone at once: the click saved it as the remembered install, and a
    // parallel test's `AppState::new` must not find it there.
    drop(dir);

    let opened = install(&state).expect("one click did not open the install");
    assert_eq!(opened.path.as_deref(), Some(found.as_path()));
    assert_eq!(state.active_document, Some(opened.id));
    assert_eq!(state.active_workspace, Workspace::Browse);
    assert_eq!(state.prefs.active_game, Some(Game::GtaVLegacy));
}

#[test]
fn switching_the_game_asks_before_it_closes_a_file_opened_from_the_old_install() {
    let dir = tempfile::tempdir().expect("an install folder");
    let mut state = AppState::from_prefs(remembering(Game::GtaVLegacy, dir.path()));
    let old = install(&state).expect("the Legacy install is open").id;
    let file = state.insert_document(
        dir.path().join("commandline.txt"),
        DocPayload::Text(TextDocument::decode(b"-windowed")),
        Some(RpfEntryBinding {
            parent_doc_id: old,
            entry_id: 1,
        }),
        None,
    );
    state.document_mut(file).expect("open").mark_edited();
    // ⚠ Not on disk: the switch saves it as the remembered install, and a
    // parallel test's `AppState::new` must not mount it.
    let enhanced = dir.path().join("enhanced");

    state.request_game_switch(Game::GtaVEnhanced, enhanced.clone());
    let pending = state
        .pending_close
        .clone()
        .expect("the switch asked nothing");
    assert_eq!(state.unsaved_documents(&pending.documents), [file]);
    assert!(
        state.document(old).is_some() && state.document(file).is_some(),
        "the switch closed something before the user answered"
    );
    state.answer_close(CloseAnswer::Cancel);
    assert!(state.document(file).is_some_and(|d| d.dirty));
    assert_eq!(
        state.prefs.active_game,
        Some(Game::GtaVLegacy),
        "Cancel switched"
    );

    state.request_game_switch(Game::GtaVEnhanced, enhanced.clone());
    state.answer_close(CloseAnswer::DontSave);
    assert!(
        state.document(old).is_none() && state.document(file).is_none(),
        "the old install or the file opened from it is still open"
    );
    let new = install(&state).expect("the Enhanced install did not open");
    assert_eq!(new.path.as_deref(), Some(enhanced.as_path()));
    assert_eq!(state.active_document, Some(new.id));
    assert_eq!(state.prefs.active_game, Some(Game::GtaVEnhanced));
}

#[test]
fn a_new_session_and_an_opened_project_keep_the_install_mounted() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let root = dir.path().join("install");
    std::fs::create_dir(&root).expect("an install folder");
    let mut state = AppState::from_prefs(Prefs::default());
    let mounted = state.open_game_root(root.clone());
    state.set_active_document(mounted);
    let notes = state.insert_document(
        dir.path().join("notes.txt"),
        DocPayload::Text(TextDocument::empty()),
        None,
        None,
    );

    state.new_project();
    assert!(state.document(notes).is_none(), "New session kept the file");
    assert_eq!(
        install(&state).map(|d| d.id),
        Some(mounted),
        "New session dropped the install"
    );
    assert_eq!(state.active_document, Some(mounted));

    let project = dir.path().join("p.openrage-proj");
    state
        .save_project_to(project.clone())
        .expect("the project saved");
    let members = load_project(&project).expect("it reads").project.documents;
    assert!(
        members.is_empty(),
        "the install is not a file of the project: {members:?}"
    );

    let older = ProjectFile {
        documents: vec![ProjectDocument::new(
            root.clone(),
            Some(DocumentKind::GameRoot),
        )],
        open: vec![0],
        ..ProjectFile::default()
    };
    save_project(&project, &older).expect("a project an older build wrote");
    state.load_project_from(project);
    assert_eq!(
        install(&state).map(|d| d.id),
        Some(mounted),
        "opening a project dropped the install"
    );
    assert_eq!(
        state.loading,
        LoadingState::Idle,
        "the install's folder was read as a file"
    );
}

fn a_project_on_disk(dir: &Path, name: &str, workspace: Workspace) -> std::path::PathBuf {
    let file = dir.join(name).join(format!("{name}.openrage-proj"));
    let project = ProjectFile {
        workspace,
        ..ProjectFile::default()
    };
    save_project(&file, &project).expect("a project on disk");
    file
}

/// The cards of `paths`, once the worker that reads them has left them.
fn cards_of(state: &mut AppState, paths: &[std::path::PathBuf]) -> Vec<super::front_door::Card> {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let cards = state.front_door.cards.of(paths);
        if cards.iter().all(Option::is_some) {
            return cards.into_iter().flatten().cloned().collect();
        }
        assert!(Instant::now() < until, "the cards were never read");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_launch_with_a_remembered_project_reopens_it_on_its_workspace() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    // The file was saved on Model; the window was left on Texture.
    let file = a_project_on_disk(dir.path(), "police", Workspace::Model);
    let prefs = Prefs {
        last_project: Some(file.clone()),
        dock_layouts: crate::ui::dock_layout::capture(
            &WorkspaceDocks::default(),
            Workspace::Texture,
        ),
        ..Prefs::default()
    };

    let mut state = AppState::from_prefs(prefs);
    state.open_from_command_line(Vec::new());

    assert_eq!(state.project_path.as_deref(), Some(file.as_path()));
    assert_eq!(
        state.active_workspace,
        Workspace::Texture,
        "the project opens on the workspace it was left on"
    );
    assert!(
        !state.front_door.raised(state.active_workspace),
        "the start page is not in the way of a project that reopened"
    );
}

#[test]
fn a_remembered_project_with_no_layout_saved_opens_on_the_workspace_its_file_states() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let file = a_project_on_disk(dir.path(), "police", Workspace::Model);
    let prefs = Prefs {
        last_project: Some(file.clone()),
        ..Prefs::default()
    };

    let mut state = AppState::from_prefs(prefs);
    state.open_from_command_line(Vec::new());

    assert_eq!(state.project_path.as_deref(), Some(file.as_path()));
    assert_eq!(state.active_workspace, Workspace::Model);
}

#[test]
fn a_launch_with_the_setting_off_opens_the_install_and_not_the_project() {
    let install_folder = tempfile::tempdir().expect("an install folder");
    let dir = tempfile::tempdir().expect("a scratch folder");
    let file = a_project_on_disk(dir.path(), "police", Workspace::Texture);
    let mut prefs = remembering(Game::GtaVLegacy, install_folder.path());
    prefs.last_project = Some(file);
    prefs.reopen_last_project = false;

    let mut state = AppState::from_prefs(prefs);
    state.open_from_command_line(Vec::new());

    assert!(install(&state).is_some(), "the remembered install is open");
    assert_eq!(state.project_path, None, "the project was reopened");
    assert_eq!(state.active_workspace, Workspace::Browse);
}

#[test]
fn a_launch_with_no_project_to_reopen_opens_the_remembered_install_in_browse() {
    let install_folder = tempfile::tempdir().expect("an install folder");
    let mut prefs = remembering(Game::GtaVLegacy, install_folder.path());
    prefs.dock_layouts =
        crate::ui::dock_layout::capture(&WorkspaceDocks::default(), Workspace::Model);

    let mut state = AppState::from_prefs(prefs);
    state.open_from_command_line(Vec::new());

    let opened = install(&state).expect("the remembered install is open");
    assert_eq!(state.active_document, Some(opened.id));
    assert_eq!(state.active_workspace, Workspace::Browse);
    assert_eq!(state.project_path, None);
    assert!(!state.front_door.raised(state.active_workspace));
}

#[test]
fn a_launch_whose_project_is_gone_shows_the_start_page_and_lists_it_as_not_found() {
    let install_folder = tempfile::tempdir().expect("an install folder");
    let dir = tempfile::tempdir().expect("a scratch folder");
    let gone = dir.path().join("moved").join("police.openrage-proj");
    let mut prefs = remembering(Game::GtaVLegacy, install_folder.path());
    prefs.touch_recent_project(gone.clone());

    let mut state = AppState::from_prefs(prefs);
    state.open_from_command_line(Vec::new());

    assert_eq!(state.project_path, None);
    assert!(install(&state).is_some(), "the install is still mounted");
    assert!(
        state.front_door.raised(state.active_workspace),
        "the start page is on screen"
    );
    let cards = cards_of(&mut state, std::slice::from_ref(&gone));
    assert!(!cards[0].found, "a project that moved is Not found");
    assert_eq!(cards[0].name, "police");
    assert!(
        state
            .toasts
            .iter()
            .all(|toast| !toast.message.contains("not found")),
        "a project in the recent list is shown on the start page and not announced twice"
    );
}

#[test]
fn closing_a_project_shows_the_start_page_and_the_next_launch_does_not_reopen_it() {
    let install_folder = tempfile::tempdir().expect("an install folder");
    let dir = tempfile::tempdir().expect("a scratch folder");
    let file = a_project_on_disk(dir.path(), "police", Workspace::Project);
    let mut state = AppState::from_prefs(remembering(Game::GtaVLegacy, install_folder.path()));
    state.load_project_from(file.clone());
    assert_eq!(state.project_path.as_deref(), Some(file.as_path()));
    assert_eq!(state.prefs.last_project.as_deref(), Some(file.as_path()));

    state.close_project();

    assert_eq!(state.project_path, None);
    assert_ne!(state.active_workspace, Workspace::Project);
    assert!(
        state.front_door.raised(state.active_workspace),
        "closing a project shows the start page"
    );
    assert!(install(&state).is_some(), "the install stays mounted");
    assert_eq!(state.prefs.last_project, None);

    let mut next = AppState::from_prefs(state.prefs.clone());
    next.open_from_command_line(Vec::new());
    assert_eq!(next.project_path, None, "a closed project was reopened");
}

#[test]
fn the_start_page_stays_up_until_a_workspace_or_a_document_is_shown() {
    let install_folder = tempfile::tempdir().expect("an install folder");
    let mut state = AppState::from_prefs(remembering(Game::GtaVLegacy, install_folder.path()));
    let mounted = install(&state).expect("the install").id;

    state.show_start_page();
    assert!(state.front_door.raised(state.active_workspace));
    state.active_workspace = Workspace::Model;
    assert!(
        !state.front_door.raised(state.active_workspace),
        "another workspace button was pressed"
    );

    // A frame on the other workspace is what tells the page it is no longer on screen.
    labelled(
        &egui::Context::default(),
        &mut state,
        Vec::new(),
        |ui, state| crate::ui::dock::draw(ui, state, None),
    );
    state.active_workspace = Workspace::Browse;
    assert!(
        !state.front_door.raised(state.active_workspace),
        "the page came back over the workspace it covered after a visit to another"
    );

    state.show_start_page();
    state.set_active_document(mounted);
    assert!(
        !state.front_door.raised(state.active_workspace),
        "a document was shown"
    );
}

#[test]
fn a_project_named_on_the_command_line_opens_and_the_last_one_does_not_reopen_beside_it() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let last = a_project_on_disk(dir.path(), "last", Workspace::Project);
    let named = a_project_on_disk(dir.path(), "named", Workspace::Model);
    let prefs = Prefs {
        last_project: Some(last),
        ..Prefs::default()
    };

    let mut state = AppState::from_prefs(prefs.clone());
    state.open_from_command_line(vec![named.clone()]);
    assert_eq!(state.project_path.as_deref(), Some(named.as_path()));
    assert_eq!(state.active_workspace, Workspace::Model);

    let folder = dir.path().join("a_folder");
    std::fs::create_dir(&folder).expect("a folder");
    let mut state = AppState::from_prefs(prefs);
    state.open_from_command_line(vec![folder.clone()]);
    assert_eq!(
        state.project_path, None,
        "a path named asked this window for something else: the last project stays closed"
    );
    assert!(state
        .documents
        .iter()
        .any(|d| d.path.as_deref() == Some(folder.as_path())));
}

#[test]
fn a_second_project_asks_whether_it_opens_here_or_in_a_new_window() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let first = a_project_on_disk(dir.path(), "first", Workspace::Project);
    let second = a_project_on_disk(dir.path(), "second", Workspace::Project);
    let mut state = AppState::from_prefs(Prefs::default());

    state.open_project_asking(first.clone());
    assert_eq!(
        state.project_path.as_deref(),
        Some(first.as_path()),
        "none was open: it opens"
    );
    state.open_project_asking(first.clone());
    assert_eq!(
        state.front_door.switching, None,
        "the same project is not a second one"
    );

    state.open_project_asking(second.clone());
    assert_eq!(
        state.front_door.switching.as_deref(),
        Some(second.as_path())
    );
    assert_eq!(
        state.project_path.as_deref(),
        Some(first.as_path()),
        "nothing opened before the answer"
    );

    state.answer_switch(Switch::Cancel);
    assert_eq!(state.project_path.as_deref(), Some(first.as_path()));
    assert!(!fake::was_opened(&second));

    state.open_project_asking(second.clone());
    state.answer_switch(Switch::NewWindow);
    assert!(fake::was_opened(&second), "a second process was asked for");
    assert_eq!(
        state.project_path.as_deref(),
        Some(first.as_path()),
        "this window keeps its project"
    );

    state.open_project_asking(second.clone());
    state.answer_switch(Switch::Here);
    assert_eq!(state.project_path.as_deref(), Some(second.as_path()));
}

#[test]
fn a_project_that_is_not_there_is_not_asked_about_and_is_listed_as_not_found() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let gone = dir.path().join("gone").join("gone.openrage-proj");
    let mut state = AppState::from_prefs(Prefs::default());

    state.open_project_asking(gone);

    assert_eq!(state.front_door.switching, None);
    assert_eq!(state.project_path, None);
    assert!(state.front_door.raised(state.active_workspace));
    assert!(
        state
            .toasts
            .iter()
            .any(|toast| toast.message.contains("moved or deleted")),
        "a project the list does not hold is announced"
    );
}

#[test]
fn removing_a_project_from_the_list_forgets_it_and_keeps_its_files() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let file = a_project_on_disk(dir.path(), "police", Workspace::Project);
    let mut prefs = Prefs::default();
    prefs.touch_recent_project(file.clone());
    let mut state = AppState::from_prefs(prefs);

    state.forget_recent_project(&file);

    assert!(state.prefs.recent_projects.is_empty());
    assert_eq!(state.prefs.last_project, None);
    assert!(file.is_file(), "Remove never deletes a file");
}

#[test]
fn locating_a_moved_project_opens_it_where_it_is_and_forgets_the_old_place() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let old = dir.path().join("old").join("police.openrage-proj");
    let moved = a_project_on_disk(dir.path(), "police", Workspace::Project);
    let mut prefs = Prefs::default();
    prefs.touch_recent_project(old.clone());
    let mut state = AppState::from_prefs(prefs);

    state.locate_recent_project(&old, moved.clone());

    assert_eq!(state.project_path.as_deref(), Some(moved.as_path()));
    assert_eq!(state.prefs.recent_projects, [moved]);
}
