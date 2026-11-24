//! The waits of a shots script: a step that opens an install, a folder or a file returns once
//! what it opened has loaded and its rows are read, so the step after it finds them.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::state::prefs::Game;
use crate::state::{AppState, LoadingState};

pub(super) const OPEN_DEADLINE: Duration = Duration::from_secs(180);

/// Make `game`'s install at `root` active as the start page does, drawing a frame through `frame` until its rows are read.
pub(crate) fn activate_game_and_wait(
    state: &mut AppState,
    game: Game,
    root: &Path,
    frame: impl FnMut(&mut AppState) -> Result<(), String>,
) -> Result<(), String> {
    // ⚠ A title read by scan has nothing to mount: its folder is set, as Settings ▸ Games sets it.
    if crate::state::gta_vi::read_by_scan(game) {
        state.prefs.game_paths.set(game, Some(root.to_path_buf()));
        return Ok(());
    }
    state.set_active_game(game, root.to_path_buf());
    wait_for_rows(state, root, frame)
}

/// Open `path` as the app does, drawing a frame through `frame` until it has loaded and its rows are read.
pub(crate) fn open_and_wait(
    state: &mut AppState,
    path: &Path,
    frame: impl FnMut(&mut AppState) -> Result<(), String>,
) -> Result<(), String> {
    if path.is_dir() {
        state.open_folder(path.to_path_buf());
    } else {
        state.load_file(path.to_path_buf());
    }
    wait_for_rows(state, path, frame)?;
    match state
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(path))
    {
        Some(doc) => println!("  opened as {:?}", doc.kind),
        // Not an error: a script may open a broken file to draw the dialog.
        None => println!("  did not open: {}", state.error_dialog_message),
    }
    Ok(())
}

/// Draw frames through `frame` until `path`'s document is no longer loading or being listed.
///
/// ⚠ Open Folder and an install's root return with their rows still being read on a worker, and
/// `loading` does not say so: a click straight after would land on "Reading this folder…".
fn wait_for_rows(
    state: &mut AppState,
    path: &Path,
    mut frame: impl FnMut(&mut AppState) -> Result<(), String>,
) -> Result<(), String> {
    let until = Instant::now() + OPEN_DEADLINE;
    loop {
        let opened = state
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path));
        let reading = opened.is_some_and(|doc| state.expander.is_listing(doc.id));
        if !reading && !matches!(state.loading, LoadingState::Loading) {
            return Ok(());
        }
        if Instant::now() > until {
            return Err(format!(
                "{} was still loading after three minutes",
                path.display()
            ));
        }
        frame(state)?;
        std::thread::sleep(Duration::from_millis(10));
    }
}
