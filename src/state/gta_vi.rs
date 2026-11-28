//! What the app says of GTA VI, whose archives it recognises by their plaintext header and cannot list by name.

use std::io::Read;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Rpf9Header, RPF9_HEADER_SIZE, RPF9_MAGIC};
use or_title::ContainerFamily;

use super::prefs::{Game, Prefs};

/// The GTA VI install the preferences hold: the active game's where it is a build of GTA VI, else the first build's with a folder set.
pub(crate) fn install(prefs: &Prefs) -> Option<(Game, &PathBuf)> {
    (prefs.active_game.into_iter().chain(Game::all()))
        .filter(|build| build.is_gta_vi())
        .find_map(|build| Some((build, prefs.game_paths.get(build)?)))
}

/// The sentence every [`rpf9_refusal`] starts with: why an RPF9 archive's entries have no names.
pub(crate) fn rpf9_why() -> &'static str {
    tr!("This is a GTA VI archive (RPF9). Its table of contents and its names are encrypted and OpenRage has no key for them, so its entries cannot be listed by name.")
}

/// The sentence [`rpf9_cut_short_at`] starts with.
fn rpf9_cut_short() -> &'static str {
    tr!("This GTA VI archive (RPF9) is cut short, so its table of contents cannot be read.")
}

/// The sentence the refusal of an RPF9 whose table does not open with the keys held starts with.
fn rpf9_unreadable() -> &'static str {
    tr!("This GTA VI archive (RPF9) has a table of contents that does not open with the GTA VI keys OpenRage holds, so its entries cannot be listed by name.")
}

/// Whether `message` is an RPF9 archive's refusal, which says why itself and leaves it to a scan, rather than an error that may only name a path holding `RPF9`.
pub(crate) fn is_rpf9_refusal(message: &str) -> bool {
    [rpf9_why(), rpf9_cut_short(), rpf9_unreadable()]
        .iter()
        .any(|lead| message.starts_with(lead))
}

/// Why the RPF9 archive at `path` is too short for its header and table of contents, with what they take and what it holds; `None` when it holds them or is not RPF9.
pub(crate) fn rpf9_cut_short_at(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let found = file.metadata().ok()?.len();
    let mut head = Vec::with_capacity(RPF9_HEADER_SIZE);
    file.take(RPF9_HEADER_SIZE as u64)
        .read_to_end(&mut head)
        .ok()?;
    if !head.starts_with(&RPF9_MAGIC.to_le_bytes()) {
        return None;
    }
    let header = RPF9_HEADER_SIZE as u64;
    let needed = Rpf9Header::parse(&head).map_or(header, |h| u64::from(h.toc_end).max(header));
    (found < needed).then(|| {
        let what = tr!(
            "Its header and table of contents take {needed} bytes, and the file holds only {found}: it may have been copied in part.",
            needed = needed,
            found = found,
        );
        format!("{}\n{what}", rpf9_cut_short())
    })
}

/// Why the RPF9 archive at `path` did not open with the reader's `error`: cut short, no key, or a table the keys held do not open; `None` when it is not RPF9.
pub(super) fn rpf9_unopened_at(path: &Path, error: &or_io::IoError) -> Option<String> {
    if let Some(short) = rpf9_cut_short_at(path) {
        return Some(short);
    }
    let refusal = rpf9_refusal_at(path)?;
    Some(match error {
        or_io::IoError::Rpf9TocEncrypted => refusal,
        other => format!("{}\n{other}", rpf9_unreadable()),
    })
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
    (Game::detect_from_root(root).into_iter().chain([game]))
        .find(|title| !title.profile().is_supported())
}

/// Why `title` is not supported, in a clause.
fn why_unsupported(title: Game) -> String {
    match title.profile().container() {
        ContainerFamily::Rpf9 => tr!("its archives are RPF9, whose tables of contents are encrypted with a key OpenRage does not have, so none of them can be listed").to_string(),
        _ => title.profile().description().to_string(),
    }
}

/// Why the folder at `root` is not opened as `game`'s install; `None` when it may be.
pub(super) fn refused_install(game: Game, root: &Path) -> Option<String> {
    let title = unsupported_title(game, root)?;
    // ⚠ A folder picked for the title and found to hold none of it is not that title: saying it is sent the person looking for a key.
    if Game::detect_from_root(root) != Some(title) {
        return Some(tr!(
            "{folder} does not look like a {title} folder, so it was not set as one.",
            folder = root.display(),
            title = title.label(),
        ));
    }
    // A title read by scan opens as its own install; picked as another title it is still refused.
    if title == game && read_by_scan(title) {
        return None;
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
    !game.profile().is_supported() && Game::detect_from_root(root) == Some(game)
}

impl super::AppState {
    /// Open `root` as `game`, a title that is not supported: one read by scan opens as the active
    /// install, any other is remembered, made active and opened as a plain folder; a folder that
    /// holds another title is refused.
    pub fn open_title_folder(&mut self, game: Game, root: PathBuf) {
        // A folder picked for one build of a title and holding another opens as the build it holds.
        let held = Game::detect_from_root(&root).filter(|held| held.title_id() == game.title_id());
        let game = held.unwrap_or(game);
        if read_by_scan(game) {
            return self.request_game_switch(game, root);
        }
        self.set_game_install_folder(game, Some(root.clone()));
        if self.prefs.game_paths.get(game) == Some(&root) {
            self.prefs.active_game = Some(game);
            super::save_prefs(&self.prefs);
            self.open_folder(root);
        }
    }
}

/// Whether `game`'s archives open as a scan of what they hold, so its folder is browsed and not refused.
pub(crate) fn read_by_scan(game: Game) -> bool {
    game.profile().container() == ContainerFamily::Rpf9
}

/// What opening the folder at `root` says when it holds a title that is not opened as an install,
/// and whether that is a warning: a title read by scan is browsed as it is meant to be.
pub(super) fn plain_folder_note(root: &Path) -> Option<(String, bool)> {
    folder_note(
        root,
        crate::state::rpf9_scan::rpf9_table::gta6_keys_held::gta6_keys,
    )
}

/// [`plain_folder_note`] with GTA VI's keys as `keys` says they stand.
fn folder_note(
    root: &Path,
    keys: impl FnOnce() -> crate::state::rpf9_scan::rpf9_table::gta6_keys_held::Gta6Keys,
) -> Option<(String, bool)> {
    let title = Game::detect_from_root(root).filter(|t| !t.profile().is_supported())?;
    if read_by_scan(title) {
        use crate::state::rpf9_scan::rpf9_table::gta6_keys_held::Gta6Keys;
        let note = match keys() {
            Gta6Keys::Asking => return None,
            Gta6Keys::Held { .. } => tr!(
                "{folder} is {title}. Its archives open from their tables as folders of the tree.",
                folder = root.display(),
                title = title.label(),
            ),
            Gta6Keys::Missing(why) => {
                let scan = tr!(
                    "{folder} is {title}. Its archives have no names OpenRage can read, so each opens as a scan of what it holds.",
                    folder = root.display(),
                    title = title.label(),
                );
                let why = tr!(
                    "No key opens the tables of these archives, so they are read by a scan: {why}",
                    why = why
                );
                format!("{scan} {why}")
            }
        };
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
            "cannot be listed".to_string(),
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
    fn opening_an_rpf9_archive_says_it_is_gta_vi_from_its_head_and_offers_no_keys() {
        let (_scratch, dir) = gta_vi_folder();
        let archive = dir.join("common.rpf");
        // ⚠ Its head alone: a table cut short is said so whether or not a key is held.
        let expected = rpf9_cut_short_at(&archive).expect("the archive is a cut-short RPF9");

        // The open of a file (a dialog, a drop, a recent file) opens it as what a scan finds.
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
        assert!(!state.show_error_dialog, "{}", state.error_dialog_message);
        let opened = state.active_document().map(|d| d.kind);
        assert_eq!(opened, Some(DocumentKind::Rpf9));
        // Its tab says why its table is not read, with a key held or none.
        state.poll_jobs();
        assert!(says(
            &state,
            ToastType::Info,
            &["common.rpf", "scan without its table", rpf9_cut_short()]
        ));

        // The archive browser's open of it as an archive.
        let crypto = state.keys.provider();
        let refused = super::super::rpf_archive_open::open_archive_from_disk(&archive, crypto)
            .map(|_| ())
            .expect_err("an RPF9 archive does not open");
        assert_eq!(refused, expected);
    }

    #[test]
    fn a_gta_vi_archive_cut_short_says_so_with_a_key_and_without() {
        let (_scratch, dir) = gta_vi_folder();
        let archive = dir.join("common.rpf");
        let found = std::fs::metadata(&archive).expect("the archive").len();
        let header = Rpf9Header::parse(&rpf9_head(418)).expect("the head parses");
        let keys = super::super::rpf_archive_open::tree_verbs::tests::synthetic_keys()
            .expect("the synthetic snapshot parses");
        let keyed = |path: &Path| {
            let mut file = std::fs::File::open(path).expect("the archive");
            or_io::container::rpf::open_at(&mut file, 0, &keys).expect_err("no table opens")
        };
        let no_key = or_io::IoError::Rpf9TocEncrypted;

        let short = rpf9_cut_short_at(&archive).expect("the archive is cut short");
        assert!(short.starts_with(rpf9_cut_short()) && is_rpf9_refusal(&short));
        for fact in [header.toc_end.to_string(), found.to_string()] {
            assert!(short.contains(&fact), "{short:?} lacks {fact:?}");
        }
        for error in [&no_key, &keyed(&archive)] {
            let said = rpf9_unopened_at(&archive, error);
            assert_eq!(said.as_deref(), Some(short.as_str()), "{error}");
        }

        // A file shorter than a header is cut short at the header.
        std::fs::write(&archive, &rpf9_head(418)[..RPF9_HEADER_SIZE / 2]).expect("write");
        let short = rpf9_cut_short_at(&archive).expect("a part of a header is cut short");
        assert!(short.contains(&RPF9_HEADER_SIZE.to_string()), "{short}");

        // Whole, it is refused for want of a key, or as a table the keys held do not open.
        let mut whole = rpf9_head(418);
        whole.resize(header.toc_end as usize, 0x5A);
        std::fs::write(&archive, whole).expect("write");
        assert_eq!(rpf9_cut_short_at(&archive), None);
        assert_eq!(
            rpf9_unopened_at(&archive, &no_key),
            rpf9_refusal_at(&archive)
        );
        let unreadable = rpf9_unopened_at(&archive, &keyed(&archive)).expect("an RPF9");
        assert!(
            unreadable.starts_with(rpf9_unreadable()) && is_rpf9_refusal(&unreadable),
            "{unreadable}"
        );

        // Another file is no RPF9, whatever its error.
        let config = dir.join(or_title::game::profile::GDK_CONFIG);
        assert_eq!(rpf9_unopened_at(&config, &no_key), None);
        assert_eq!(rpf9_cut_short_at(&config), None);
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

        // Opened as a folder, it is browsed with no warning, and says how its archives open as the
        // keys stand.
        state.open_folder(dir.clone());
        assert!(!says(&state, ToastType::Warning, &["GTA VI"]));
        use crate::state::rpf9_scan::rpf9_table::gta6_keys_held::Gta6Keys;
        let note = |keys: Gta6Keys| folder_note(&dir, move || keys);
        let scan = note(Gta6Keys::Missing("zq why".into())).expect("a note");
        assert!(!scan.1 && scan.0.contains("opens as a scan") && scan.0.contains("zq why"));
        let held = note(Gta6Keys::Held { names: Some(2) }).expect("a note");
        assert!(
            !held.1 && held.0.contains("folders of the tree"),
            "{}",
            held.0
        );
        assert_eq!(
            note(Gta6Keys::Asking),
            None,
            "a neutral toast while the keys are asked"
        );

        // A game's folder is on screen with no project open, and is not offered as one.
        let on_screen = state.active_document().map(|d| (d.kind, d.path.clone()));
        assert_eq!(on_screen, Some((DocumentKind::Folder, Some(dir))));
        assert_eq!(state.project_path, None);
        assert_eq!(state.adoption_offer(), None);
    }

    #[test]
    fn going_into_a_gta_vi_archive_opens_its_scan_and_back_does_not_lead_into_it_again() {
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
        let scan_is_open =
            |state: &AppState| state.active_document().map(|d| d.kind) == Some(DocumentKind::Rpf9);
        frames(&mut state, &scan_is_open, "the archive's scan never opened");
        assert_eq!(state.navigation.current(), Some(Location::document(folder)));

        // The folder's tab shows the folder again, and stays on screen.
        state.set_active_document(folder);
        for _ in 0..3 {
            state.poll_load_result();
            frame(&ctx, &mut state, Vec::new());
        }
        assert_eq!(state.active_document, Some(folder));
        assert!(lists_it(&state), "{:?}", listed(&state, folder));
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
        state.open_title_folder(Game::GtaVI, dir.clone());
        assert_eq!(state.prefs.game_paths.get(Game::GtaVI), Some(&dir));
        assert_eq!(state.prefs.active_game, Some(Game::GtaVI));
        let kinds: Vec<_> = (state.documents.iter())
            .map(|d| (d.kind, d.path.clone()))
            .collect();
        assert_eq!(kinds, [(DocumentKind::GameRoot, Some(dir.clone()))]);

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

    #[test]
    fn a_playstation_5_folder_opens_as_its_own_build_and_is_the_gta_vi_install() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dir = scratch.path().to_path_buf();
        std::fs::create_dir(dir.join("sce_sys")).expect("sce_sys");
        let params = br#"{"titleId": "PPSA38500"}"#;
        std::fs::write(dir.join("sce_sys/param.json"), params).expect("params");
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        assert_eq!(install(&state.prefs), None);

        // Picked on the Xbox build's row, it is still the build its folder holds.
        state.open_title_folder(Game::GtaVI, dir.clone());
        assert_eq!(state.prefs.game_paths.get(Game::GtaVI), None);
        assert_eq!(state.prefs.game_paths.get(Game::GtaVIPs5), Some(&dir));
        assert_eq!(state.prefs.active_game, Some(Game::GtaVIPs5));
        assert_eq!(install(&state.prefs), Some((Game::GtaVIPs5, &dir)));
        assert!(!says(&state, ToastType::Error, &["GTA VI"]));
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
    fn a_gta_vi_archive_is_drawn_in_the_tree_as_one_that_opens_and_says_it_opens_as_a_scan() {
        use super::super::nothing_draws_over_what_it_belongs_to::Panel;
        use super::super::{DocPayload, RpfNodeBacking};
        use crate::ui::theme;
        let _serial = theme::serial();
        let (_scratch, dir) = gta_vi_folder();
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        state.open_folder(dir);
        let folder = state.active_document.expect("the folder opens");

        // The archive's row once its open as an archive was refused, as a click on it leaves it.
        let deadline = Instant::now() + Duration::from_secs(30);
        let row = loop {
            assert!(Instant::now() < deadline, "the archive was never refused");
            state.poll_expansions();
            let Some(DocPayload::Rpf(rpf)) = state.document(folder).map(|d| &d.payload) else {
                panic!("the folder's tree");
            };
            let rows = rpf.session.children_of(rpf.session.root_id());
            let mut rows = rows.iter().filter_map(|id| rpf.session.node(*id));
            let refused = |backing: &RpfNodeBacking| matches!(backing, RpfNodeBacking::Archive { open_error: Some(why), .. } if is_rpf9_refusal(why));
            let row = rows.find(|row| row.name == "common.rpf").cloned();
            match row {
                Some(row) if refused(&row.backing) => break row,
                Some(row) => state.request_expand(folder, row.id),
                None => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let icon = crate::ui::icons::rpf_node_icon(&row, false, false, true, false);
        assert_eq!(icon.badge, None, "badged as an archive that did not open");

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
        assert_eq!(*colour, theme::TEXT_PRIMARY.get(), "its name is dimmed");

        // Held still for long enough for a tooltip.
        let mut said = frame(&mut state, vec![egui::Event::PointerMoved(at.center())]);
        for _ in 0..45 {
            said = frame(&mut state, Vec::new());
        }
        let tip = (said.iter()).find(|(text, ..)| text.contains("It opens as a scan"));
        let (tip, ..) = tip.unwrap_or_else(|| panic!("the row says no such thing: {said:?}"));
        assert!(is_rpf9_refusal(tip), "{tip}");
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
