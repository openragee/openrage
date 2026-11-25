//! What the app says of GTA VI archives and folders.

use std::io::Read;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Rpf9Header, RPF9_HEADER_SIZE};
use or_title::ContainerFamily;

use super::prefs::Game;

/// The sentence every [`rpf9_refusal`] starts with: why RPF9 opens in its own tab.
pub(crate) fn rpf9_why() -> &'static str {
    tr!("This is a GTA VI archive (RPF9). Open it in its own tab to read its table with the GTA VI keys.")
}

/// Whether `message` is an [`rpf9_refusal`], which says why itself, rather than an error that may only name a path holding `RPF9`.
pub(crate) fn is_rpf9_refusal(message: &str) -> bool {
    message.starts_with(rpf9_why())
}

/// Why the archive whose head is `head` is not opened, with what its header says; `None` when it is not RPF9.
pub(super) fn rpf9_refusal(head: &[u8]) -> Option<String> {
    let header = Rpf9Header::parse(head).ok()?;
    Some(format!("{}\n{}", rpf9_why(), rpf9_facts(&header)))
}

/// What an RPF9 header states, in a sentence.
pub(crate) fn rpf9_facts(header: &Rpf9Header) -> String {
    let per_archive: String = header
        .per_archive
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let counts = tr_n!(
        "Its header says: {count} entry, a table of contents ending at byte {toc_end}, {names} bytes of names, {width}-byte entry indices, and the per-archive value {per_archive}, whose role is not known.",
        "Its header says: {count} entries, a table of contents ending at byte {toc_end}, {names} bytes of names, {width}-byte entry indices, and the per-archive value {per_archive}, whose role is not known.",
        header.entry_count,
        toc_end = format!("{:#X}", header.toc_end),
        names = header.names_size,
        width = header.index_width,
        per_archive = per_archive,
    );
    let parts = tr!(
        "The table's parts start at 0x40, {part_b} and {part_c}, and its index at {index}; its layout byte is {layout} and its index kind {kind}. Kind 1 indexes the entry rows alone; what the layout byte and the other kinds select is not known.",
        part_b = format!("{:#X}", 0x40 + u64::from(header.part_b_offset)),
        part_c = format!("{:#X}", 0x40 + u64::from(header.part_c_offset)),
        index = format!("{:#X}", header.index_offset),
        layout = header.layout,
        kind = header.index_kind,
    );
    format!("{counts} {parts}")
}

/// [`rpf9_refusal`] for the file at `path`, of which only the header is read.
pub(super) fn rpf9_refusal_at(path: &Path) -> Option<String> {
    let mut head = [0u8; RPF9_HEADER_SIZE];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .ok()?;
    rpf9_refusal(&head)
}

/// The title the folder at `root` holds, or else `game`, when OpenRage cannot read that title yet.
pub(super) fn unsupported_title(game: Game, root: &Path) -> Option<Game> {
    [Game::detect_from_root(root), game]
        .into_iter()
        .find(|title| !title.profile().is_supported())
}

/// Why `title` is not supported, in a clause.
fn why_unsupported(title: Game) -> String {
    match title.profile().container() {
        ContainerFamily::Rpf9 => tr!("its RPF9 archives open individually from their tables; mounting a whole install is not supported yet").to_string(),
        _ => title.profile().description().to_string(),
    }
}

/// Why the folder at `root` is not opened as `game`'s install; `None` when it may be.
pub(super) fn refused_install(game: Game, root: &Path) -> Option<String> {
    let title = unsupported_title(game, root)?;
    // ⚠ A folder picked for the title and found to hold none of it is not that title: saying it is sent the person looking for a key.
    if Game::detect_from_root(root) != title {
        return Some(tr!(
            "{folder} does not look like a {title} folder, so it was not set as one.",
            folder = root.display(),
            title = title.label(),
        ));
    }
    Some(tr!(
        "{folder} is {title}, which OpenRage does not support yet, so it was not opened as an install: {why}.",
        folder = root.display(),
        title = title.label(),
        why = why_unsupported(title),
    ))
}

/// Whether `root` holds `game`, a title OpenRage cannot open as an install yet, whose folder is
/// remembered and opened as a plain folder.
pub(super) fn holds_unsupported(game: Game, root: &Path) -> bool {
    !game.profile().is_supported() && Game::detect_from_root(root) == game
}

impl super::AppState {
    /// Remember `root` as the folder of `game`, a title OpenRage cannot open as an install yet, and
    /// open it as a plain folder; a folder that holds another title is refused.
    pub fn open_title_folder(&mut self, game: Game, root: PathBuf) {
        self.set_game_install_folder(game, Some(root.clone()));
        if self.prefs.game_paths.get(game) == Some(&root) {
            self.open_folder(root);
        }
    }
}

/// Whether `game`'s archives open individually, so its folder is browsed and not refused.
pub(crate) fn opens_archives_individually(game: Game) -> bool {
    game.profile().container() == ContainerFamily::Rpf9
}

/// What opening the folder at `root` says when it holds a title that is not opened as an install,
/// and whether that is a warning: a title with individual archives is browsed as it is meant to be.
pub(super) fn plain_folder_note(root: &Path) -> Option<(String, bool)> {
    let title = Some(Game::detect_from_root(root)).filter(|t| !t.profile().is_supported())?;
    if opens_archives_individually(title) {
        let note = tr!(
            "{folder} is {title}. Each archive opens from its table using the GTA VI keys.",
            folder = root.display(),
            title = title.label(),
        );
        return Some((note, false));
    }
    let note = tr!(
        "{folder} is {title}, which OpenRage does not support yet, so it is shown as a plain folder: {why}.",
        folder = root.display(),
        title = title.label(),
        why = why_unsupported(title),
    );
    Some((note, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::RPF9_MAGIC;

    /// An RPF9 head of layout 4 that holds every measured rule, with arbitrary values at the
    /// offsets the reader takes its facts from.
    fn rpf9_head(entries: u32) -> Vec<u8> {
        let width = if entries > 0xFF { 2 } else { 1 };
        let index = 0x1A0u32;
        let toc_end = index + (entries * (8 + width)).next_multiple_of(16);
        let mut head = vec![0u8; RPF9_HEADER_SIZE];
        for (at, word) in [
            (0x00, RPF9_MAGIC),
            (0x04, toc_end),
            (0x08, index),
            (0x0C, 0x30),
            (0x1C, 0x150),
            (0x24, entries),
            (0x30, 0x10),
        ] {
            head[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        head[0x10..0x18].copy_from_slice(&[0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6, 0x07, 0x18]);
        (head[0x19], head[0x1A], head[0x20], head[0x22]) = (0x7A, 4, 5, 1);
        (head[0x2C], head[0x2D], head[0x2E]) = (width as u8, 0x0F, 0x09);
        head
    }

    #[test]
    fn an_rpf9_head_is_named_as_gta_vi_with_its_header_facts_and_why() {
        let head = rpf9_head(418);
        let header = Rpf9Header::parse(&head).expect("the head parses");
        let said = rpf9_refusal(&head).expect("an RPF9 head is refused by name");
        for fact in [
            "GTA VI".to_string(),
            "read its table".to_string(),
            format!("{} entries", header.entry_count),
            format!("{:#X}", header.toc_end),
            format!("{} bytes of names", header.names_size),
            format!("{}-byte", header.index_width),
            header
                .per_archive
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            format!("at 0x40, {:#X}", 0x40 + header.part_b_offset),
            format!("and {:#X}", 0x40 + header.part_c_offset),
            format!("its index at {:#X}", header.index_offset),
            format!("layout byte is {}", header.layout),
            format!("index kind {}", header.index_kind),
        ] {
            assert!(said.contains(&fact), "{said:?} lacks {fact:?}");
        }
        assert!(rpf9_refusal(&rpf9_head(1)).is_some_and(|s| s.contains("1 entry,")));
    }

    #[test]
    fn a_refusal_is_told_by_its_sentence_and_not_by_a_path_that_names_rpf9() {
        let refusal = rpf9_refusal(&rpf9_head(3)).expect("an RPF9 head is refused");
        assert!(is_rpf9_refusal(&refusal));
        let elsewhere = format!(
            "open '{}': {}",
            Path::new("RPF9_backup").join("dlc.rpf").display(),
            or_io::IoError::RpfBadMagic { found: 0 }
        );
        assert!(!is_rpf9_refusal(&elsewhere), "{elsewhere:?}");
    }

    #[test]
    fn another_archive_or_a_short_head_is_not_rpf9() {
        let mut rpf7 = rpf9_head(3);
        rpf7[..4].copy_from_slice(&or_io::container::rpf::RPF7_MAGIC.to_le_bytes());
        assert_eq!(rpf9_refusal(&rpf7), None);
        assert_eq!(rpf9_refusal(&rpf9_head(3)[..RPF9_HEADER_SIZE - 1]), None);
    }

    use super::super::{AppState, DocumentKind, Prefs, ToastType};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    /// A scratch folder laid out as a GTA VI package is: a title configuration naming GTA VI, and an RPF9 `common.rpf` with bytes past its head.
    fn gta_vi_folder() -> (tempfile::TempDir, PathBuf) {
        let scratch = tempfile::Builder::new()
            .prefix("openrage-gta-vi-")
            .tempdir()
            .expect("scratch");
        let dir = scratch.path().to_path_buf();
        let config = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Game configVersion=\"1\">\n  <TitleId>{}</TitleId>\n</Game>\n",
            Game::GtaVI.profile().gdk_title_id().expect("GTA VI's title id")
        );
        std::fs::write(dir.join(or_title::game::profile::GDK_CONFIG), config).expect("config");
        let mut archive = rpf9_head(418);
        archive.resize(0x1A0 + 0x30, 0x5A);
        std::fs::write(dir.join("common.rpf"), archive).expect("archive");
        (scratch, dir)
    }

    fn says(state: &AppState, kind: ToastType, words: &[&str]) -> bool {
        (state.toasts.iter())
            .filter(|t| t.toast_type == kind)
            .any(|t| words.iter().all(|w| t.message.contains(w)))
    }

    #[test]
    fn an_unreadable_rpf9_table_reports_an_error_without_asking_for_gta_v_keys() {
        let (_scratch, dir) = gta_vi_folder();
        let archive = dir.join("common.rpf");
        let expected = rpf9_refusal_at(&archive).expect("the archive is RPF9");

        // This header carries no readable table; normal opening must not recover by scanning.
        let mut state = AppState::from_prefs(Prefs::default());
        assert!(!state.maybe_defer_encrypted_rpf(&archive));
        assert!(
            !state.keys.show_prompt,
            "a GTA VI archive asked for GTA V keys"
        );
        state.load_file(archive.clone());
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.load_receiver.is_some() || !state.pending_loads.is_empty() {
            state.poll_load_result();
            assert!(Instant::now() < deadline, "the load never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.show_error_dialog, "an unreadable table opened");
        assert!(
            state.error_dialog_plain.is_none(),
            "the table error was hidden"
        );
        let opened = state.active_document().map(|d| d.kind);
        assert_eq!(opened, None);

        // The archive browser's open of it as an archive.
        let crypto = state.keys.provider();
        let refused = super::super::rpf_archive_open::open_archive_from_disk(&archive, crypto)
            .map(|_| ())
            .expect_err("an RPF9 archive does not open");
        assert_eq!(refused, expected);
    }

    #[test]
    fn a_gta_vi_folder_is_named_and_never_opened_as_gta_v() {
        let (_scratch, dir) = gta_vi_folder();
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        let before = state.prefs.active_game;

        state.request_game_switch(Game::GtaVLegacy, dir.clone());
        state.set_active_game(Game::GtaVLegacy, dir.clone());
        state.set_game_install_folder(Game::GtaVLegacy, Some(dir.clone()));
        assert!(
            !state
                .documents
                .iter()
                .any(|d| d.kind == DocumentKind::GameRoot),
            "a GTA VI folder was mounted"
        );
        assert_eq!(state.prefs.game_paths.get(Game::GtaVLegacy), None);
        assert_eq!(state.prefs.active_game, before);
        assert!(says(
            &state,
            ToastType::Error,
            &["GTA VI", "does not support yet", "RPF9"]
        ));

        // A launch on preferences that already name the folder.
        let mut prefs = Prefs::default();
        prefs.game_paths.set(Game::GtaVLegacy, Some(dir.clone()));
        prefs.active_game = Some(Game::GtaVLegacy);
        let launched = AppState::from_prefs(prefs);
        assert!(!launched
            .documents
            .iter()
            .any(|d| d.kind == DocumentKind::GameRoot));
        assert!(says(
            &launched,
            ToastType::Error,
            &["GTA VI", "does not support yet"]
        ));

        // A file inside it is a file of GTA VI's install, as the write gate sees it.
        let prefs = Prefs::default();
        assert_eq!(
            super::super::retail::game_of(&dir.join("common.rpf"), &prefs),
            Some(Game::GtaVI)
        );

        // Opened as a folder, it is browsed and says how its archives open, with no warning.
        state.open_folder(dir.clone());
        assert!(says(
            &state,
            ToastType::Info,
            &["GTA VI", "opens from its table"]
        ));
        assert!(!says(&state, ToastType::Warning, &["GTA VI"]));

        // A game's folder is on screen with no project open, and is not offered as one.
        let on_screen = state.active_document().map(|d| (d.kind, d.path.clone()));
        assert_eq!(on_screen, Some((DocumentKind::Folder, Some(dir))));
        assert_eq!(state.project_path, None);
        assert_eq!(state.adoption_offer(), None);
    }

    #[test]
    fn an_unreadable_gta_vi_archive_keeps_the_folder_and_back_does_not_retry_it() {
        use super::super::navigation::Location;
        use super::super::the_files_view_in_the_centre::{frame, listed, node};
        let _serial = crate::ui::theme::serial();
        let (_scratch, dir) = gta_vi_folder();
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        state.prefs.explorer = Default::default();
        state.open_folder(dir.clone());
        let folder = state.active_document.expect("the folder opens");
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        let frames = |state: &mut AppState, until: &dyn Fn(&AppState) -> bool, what: &str| {
            while !until(state) {
                assert!(Instant::now() < deadline, "{what}");
                state.poll_expansions();
                state.poll_load_result();
                frame(&ctx, state, Vec::new());
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let lists_it = |state: &AppState| listed(state, folder).iter().any(|n| n == "common.rpf");
        frames(&mut state, &lists_it, "the folder never listed its archive");

        // As a double click on its row goes into it.
        let archive = node(&state, folder, "common.rpf");
        state.navigation.visit(Location::document(folder));
        state.navigate_to(Location {
            doc: folder,
            node: archive,
        });
        let failed = |state: &AppState| {
            let Some(super::super::DocPayload::Rpf(rpf)) =
                state.document(folder).map(|doc| &doc.payload)
            else {
                return false;
            };
            matches!(
                rpf.session.node(archive).map(|node| &node.backing),
                Some(
                    super::super::RpfNodeBacking::Archive {
                        open_error: Some(_),
                        ..
                    } | super::super::RpfNodeBacking::Rpf9Archive {
                        open_error: Some(_),
                        ..
                    }
                )
            )
        };
        frames(
            &mut state,
            &failed,
            "the unreadable table never reported its error",
        );
        assert!(!state
            .documents
            .iter()
            .any(|doc| doc.kind == DocumentKind::Rpf9));
        assert!(
            !state.show_error_dialog,
            "the error belongs to the archive row"
        );
        assert_eq!(
            state.navigation.current(),
            Some(Location {
                doc: folder,
                node: archive
            })
        );
        assert!(
            state.navigate_back(),
            "Back returns to the containing folder"
        );
        assert_eq!(state.navigation.current(), Some(Location::document(folder)));

        // The folder's tab shows the folder again, and stays on screen.
        state.set_active_document(folder);
        for _ in 0..3 {
            state.poll_load_result();
            frame(&ctx, &mut state, Vec::new());
        }
        assert_eq!(state.active_document, Some(folder));
        assert!(lists_it(&state), "{:?}", listed(&state, folder));
        assert!(!state.expander.is_in_flight(folder, archive));
        assert!(!state.navigate_back(), "Back led into the archive again");
    }

    /// The text of every shape `draw` paints.
    fn painted_by(state: &mut AppState, draw: fn(&mut egui::Ui, &mut AppState)) -> String {
        use super::super::nothing_draws_over_what_it_belongs_to::{painted, Panel};
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 1400.0));
        let mut panel = Panel::new(rect);
        let mut output = None;
        for _ in 0..2 {
            output = Some(panel.frame(Vec::new(), |ui| draw(ui, state)));
        }
        let texts: Vec<String> = (painted(&output.expect("drawn")).into_iter())
            .map(|p| p.text)
            .collect();
        texts.join("\n")
    }

    #[test]
    fn a_gta_vi_folder_is_remembered_as_gta_vi_and_offered_to_open() {
        let _serial = crate::ui::theme::serial();
        let (_scratch, dir) = gta_vi_folder();
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        let before = state.prefs.active_game;
        state.open_title_folder(Game::GtaVI, dir.clone());
        assert_eq!(state.prefs.game_paths.get(Game::GtaVI), Some(&dir));
        assert_eq!(state.prefs.active_game, before);
        let kinds: Vec<_> = (state.documents.iter())
            .map(|d| (d.kind, d.path.clone()))
            .collect();
        assert_eq!(kinds, [(DocumentKind::Folder, Some(dir.clone()))]);

        for draw in [
            crate::ui::panels::game_picker::draw_start_page,
            crate::ui::panels::settings_games::draw_games_page,
        ] {
            let said = painted_by(&mut state, draw);
            assert!(said.lines().any(|line| line == "Open"), "{said}");
            for denial in ["Open as a folder", "not readable yet"] {
                assert!(!said.contains(denial), "{denial:?} in:\n{said}");
            }
        }

        // With no install open, the status bar names the game the tab on screen belongs to.
        state.prefs.active_game = None;
        let said = painted_by(&mut state, crate::ui::panels::status_bar::draw);
        assert!(
            said.contains("GTA VI") && !said.contains("No game"),
            "{said}"
        );

        // A folder that holds no GTA VI is not remembered as GTA VI.
        let other = tempfile::tempdir().expect("scratch");
        state.open_title_folder(Game::GtaVI, other.path().to_path_buf());
        assert_eq!(state.prefs.game_paths.get(Game::GtaVI), Some(&dir));
    }

    /// Each text `output` painted, where, and the colour of its last run: a row's name.
    fn texts(output: &egui::FullOutput) -> Vec<(String, egui::Rect, egui::Color32)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect, egui::Color32)>) {
            match shape {
                egui::Shape::Text(text) => {
                    let run = text.galley.job.sections.last();
                    let colour = run.map_or(egui::Color32::PLACEHOLDER, |run| run.format.color);
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    out.push((text.galley.text().to_string(), rect, colour));
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        (output.shapes.iter()).for_each(|clipped| walk(&clipped.shape, &mut out));
        out
    }

    #[test]
    fn an_unreadable_gta_vi_archive_shows_its_error_in_the_files_tree() {
        use super::super::nothing_draws_over_what_it_belongs_to::Panel;
        use super::super::{DocPayload, RpfNodeBacking};
        use crate::ui::theme;
        let _serial = theme::serial();
        let (_scratch, dir) = gta_vi_folder();
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        state.open_folder(dir);
        let folder = state.active_document.expect("the folder opens");

        let deadline = Instant::now() + Duration::from_secs(30);
        let row = loop {
            assert!(Instant::now() < deadline, "the archive was never refused");
            state.poll_expansions();
            let Some(DocPayload::Rpf(rpf)) = state.document(folder).map(|d| &d.payload) else {
                panic!("the folder's tree");
            };
            let rows = rpf.session.children_of(rpf.session.root_id());
            let mut rows = rows.iter().filter_map(|id| rpf.session.node(*id));
            let refused = |backing: &RpfNodeBacking| {
                matches!(
                    backing,
                    RpfNodeBacking::Archive {
                        open_error: Some(_),
                        ..
                    } | RpfNodeBacking::Rpf9Archive {
                        open_error: Some(_),
                        ..
                    }
                )
            };
            let row = rows.find(|row| row.name == "common.rpf").cloned();
            match row {
                Some(row) if refused(&row.backing) => break row,
                Some(row) => state.request_expand(folder, row.id),
                None => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let icon = crate::ui::icons::rpf_node_icon(&row, false, false, true, false);
        assert_eq!(icon.badge, Some(crate::ui::icons::BadgeKind::Error));
        let error = match &row.backing {
            RpfNodeBacking::Archive {
                open_error: Some(error),
                ..
            }
            | RpfNodeBacking::Rpf9Archive {
                open_error: Some(error),
                ..
            } => error,
            _ => unreachable!("the row failed to open"),
        };
        assert!(!is_rpf9_refusal(error));

        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 900.0));
        let mut panel = Panel::new(area);
        let mut frame = |state: &mut AppState, events: Vec<egui::Event>| {
            texts(&panel.frame(events, |ui| crate::ui::panels::hierarchy::draw(ui, state)))
        };
        for _ in 0..2 {
            frame(&mut state, Vec::new());
        }
        let drawn = frame(&mut state, Vec::new());
        let (_, at, colour) = (drawn.iter())
            .find(|(text, ..)| text.ends_with("common.rpf"))
            .unwrap_or_else(|| panic!("the row is not drawn: {drawn:?}"));
        assert_eq!(*colour, theme::TEXT_SECONDARY.get());

        // Held still for long enough for a tooltip.
        let mut said = frame(&mut state, vec![egui::Event::PointerMoved(at.center())]);
        for _ in 0..45 {
            said = frame(&mut state, Vec::new());
        }
        assert!(
            said.iter().any(|(text, ..)| text.contains(error)),
            "{said:?}"
        );
        assert!(!said
            .iter()
            .any(|(text, ..)| text.contains("in a tab of its own")));
    }

    #[test]
    fn a_refused_switch_leaves_the_install_that_is_open_open() {
        let (_scratch, dir) = gta_vi_folder();
        let gta5 = tempfile::tempdir().expect("scratch");
        std::fs::write(gta5.path().join("GTA5.exe"), b"").expect("marker");
        let mut state = AppState::from_prefs(Prefs::default());
        let open = state.open_game_root(gta5.path().to_path_buf());

        state.request_game_switch(Game::GtaVLegacy, dir);
        assert!(
            state.document(open).is_some(),
            "the open install was closed for a switch that was refused"
        );
        assert!(
            state.pending_close.is_none(),
            "a refused switch asked about unsaved work"
        );
        assert!(says(
            &state,
            ToastType::Error,
            &["GTA VI", "does not support yet"]
        ));
    }
}
