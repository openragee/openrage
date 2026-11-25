//! The Project workspace's panels as they are drawn: how they count, what the quick look shows and where it
//! sits, the filter, the rows of Problems, a missing member and an id two folders share, and what a narrow
//! window keeps, read through AccessKit from whole frames of the shell.

use super::frames_of_the_shell::Shell;
use super::the_project_screens_on_screen::{a_project_shell, shows, until, write};
use crate::state::front_door::cards::read_card;
use crate::state::ThemeChoice;
use crate::ui::dock::Workspace;

/// Whether some widget says `text`, whatever breaks its layout put between the words.
fn says(shell: &Shell, text: &str) -> bool {
    let bare = |text: &str| text.split_whitespace().collect::<String>();
    let wanted = bare(text);
    super::the_project_screens_on_screen::names(shell)
        .iter()
        .any(|name| bare(name).contains(&wanted))
}

/// The clothing project of the screens' tests with a source folder that no member names: the walk finds it and the table lists it.
fn a_project_with_a_source(dir: &std::path::Path) -> Shell {
    let mut shell = a_project_shell(dir, ThemeChoice::Dark);
    let folder = or_project::project_folder(&shell.state.project_path.clone().expect("a project"));
    write(&folder.join("sources/logo.textures.src/asset.json"), b"{}");
    // The card reads the file the project was last saved as.
    assert!(shell.state.save_active_project(|_| None));
    shell.state.workbench.want_scan();
    until(&mut shell, "the source's row", |s| {
        s.state
            .workbench
            .rows()
            .iter()
            .any(|row| row.name == "logo")
    });
    shell.state.active_workspace = Workspace::Project;
    shell.settled();
    shell
}

#[test]
fn the_card_the_content_panel_and_properties_count_one_way_with_real_plurals() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_with_a_source(dir.path());
    let project = shell.state.project_path.clone().expect("a project");
    let files = shell
        .state
        .workbench
        .rows()
        .iter()
        .filter(|r| r.kept != crate::state::workbench::Kept::Pack)
        .count();
    let garments = shell.state.workbench.rows().len() - files;
    assert_eq!(
        (files, garments),
        (4, 3),
        "three members and a source, and three garments"
    );

    // The card reads the project file on a worker and counts what the table lists.
    let card = read_card(&project);
    assert!(
        card.content.contains(&format!(
            "Files · {}",
            crate::state::front_door::counted(files, "file")
        )),
        "the card counts the source folder the table lists: {:?}",
        card.content
    );
    assert!(
        card.content.contains(&format!(
            "Clothing · {}",
            crate::state::front_door::counted(garments, "garment")
        )),
        "{:?}",
        card.content
    );

    // The panel says the same numbers, with the plural a number takes.
    assert!(
        shows(&shell, &format!("Files  ·  {files} assets")),
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(shows(&shell, &format!("Clothing  ·  {garments} garments")));

    // Properties of the project says them again, and counts the problems with their words.
    shell.state.select_asset(None);
    shell.settled();
    assert!(shows(&shell, &format!("{} assets", files + garments)));
    assert!(shows(&shell, &format!("Files · {files} assets")));
    let errors = shell
        .state
        .workbench
        .findings()
        .iter()
        .filter(|f| f.severity == crate::state::workbench::Severity::Error)
        .count();
    assert!(errors > 0, "the project has a missing file");
    assert!(
        shows(&shell, &format!("{errors} error")) && !shows(&shell, "(s)"),
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
}

/// A textures source `name` in the project's `sources/` holding one 8 by 4 PNG called `logo.png`, and the folder.
fn a_source_of_one_png(project_folder: &std::path::Path, name: &str) -> std::path::PathBuf {
    use or_source::asset::{folder_name, AssetManifest, How, Origin, MANIFEST};
    let dir = project_folder
        .join("sources")
        .join(folder_name(name, or_source::textures::KIND));
    let origin = Origin {
        how: How::Authored,
        from_game: false,
        virtual_path: None,
        game_build: None,
        sha256: None,
        cooked_sha256: None,
        extra: Default::default(),
    };
    let manifest =
        AssetManifest::new(or_source::textures::KIND, name, "gen8", origin).expect("a manifest");
    write(&dir.join(MANIFEST), manifest.to_text().as_bytes());
    write(
        &dir.join(or_source::textures::RULES),
        br#"{"textures": []}"#,
    );
    let mut image = or_io::codec::texture::RgbaImage::new(8, 4);
    for y in 0..4 {
        for x in 0..8 {
            image.set_pixel(x, y, [(x * 32) as u8, (y * 64) as u8, 17, 255]);
        }
    }
    let png = or_io::codec::texture::encode_png(&image).expect("a PNG");
    write(&dir.join("textures/logo.png"), &png);
    dir
}

/// Frames of a window of `size` until `done`.
fn until_sized(shell: &mut Shell, size: (u32, u32), what: &str, done: impl Fn(&Shell) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        shell.state.poll_workbench();
        shell.draw_sized(size, Vec::new());
        if done(shell) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what} never happened: {:?}",
            super::the_project_screens_on_screen::names(shell)
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Every control of Properties that acts on the selected asset.
const PROPERTIES_BUTTONS: [&str; 3] = ["Open in", "Reveal asset", "Compile again"];

#[test]
fn the_quick_look_draws_the_texture_and_says_what_the_asset_is_and_never_covers_its_buttons() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    let folder = or_project::project_folder(&shell.state.project_path.clone().expect("a project"));
    a_source_of_one_png(&folder, "logo");
    shell.state.workbench.want_scan();
    until(&mut shell, "the source's row", |s| {
        s.state
            .workbench
            .rows()
            .iter()
            .any(|row| row.name == "logo")
    });
    let key = (shell.state.workbench.rows().iter())
        .find(|row| row.name == "logo")
        .expect("the source's row")
        .key
        .clone();
    shell.state.active_workspace = Workspace::Project;
    shell.state.select_asset(Some(key));
    shell.state.workbench.quick_look = true;
    let size = (900, 700);
    until_sized(&mut shell, size, "the picture", |s| {
        shows(s, "Picture of logo.png")
    });

    // Properties states which files the texture compiles from, and the quick look says them too in a line of its own, so an asset can be known without Properties: what it is, and the textures it holds.
    let named = |shell: &Shell, name: &str| {
        (shell.described.iter())
            .filter(|w| w.name.as_deref() == Some(name))
            .count()
    };
    assert!(
        says(&shell, "logo.png (image)"),
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert_eq!(
        named(&shell, "Textures: logo.png"),
        1,
        "the quick look lists the textures of the source: {:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(
        says(&shell, "Textures source  ·  "),
        "the quick look says what the asset is: {:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert_eq!(
        named(&shell, "logo.png, 8 by 4"),
        1,
        "the quick look captions the picture with its file and size"
    );

    // ⚠ The quick look sits in the Assets table's panel: nothing of it is where a button of Properties is.
    let look: Vec<egui::Rect> = (shell.described.iter())
        .filter(|w| {
            w.name.as_deref().is_some_and(|n| {
                n.starts_with("Quick look")
                    || n.starts_with("Picture of")
                    || n == "Close quick look"
            })
        })
        .map(|w| w.bounds)
        .collect();
    assert!(
        look.len() >= 3,
        "the quick look draws its title, its picture and its close button: {look:?}"
    );
    for button in PROPERTIES_BUTTONS {
        let at = (shell.described.iter())
            .find(|w| w.name.as_deref().is_some_and(|n| n.starts_with(button)))
            .unwrap_or_else(|| panic!("Properties has no `{button}`"))
            .bounds;
        assert!(
            look.iter().all(|rect| !rect.intersects(at)),
            "the quick look covers `{button}` at {at:?}: {look:?}"
        );
    }
}

#[test]
fn the_ux_design_says_the_rows_and_the_quick_look_the_panels_draw() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/direction/Workbench_UX_Design.md");
    let doc = std::fs::read_to_string(path).expect("the UX design");
    // A content reads `word · what it holds` whether it is linked or not, and an addon ped content counts its peds.
    for drawn in ["`Clothing · 1 garment`", "`Addon ped · 2 peds`"] {
        assert!(
            doc.contains(drawn),
            "the design does not say the row `{drawn}`"
        );
    }
    for stale in [
        "· link`",
        "ped(s)",
        "the textures a source or a garment holds",
    ] {
        assert!(!doc.contains(stale), "the design still says `{stale}`");
    }
}

#[test]
fn the_ux_design_says_one_way_that_open_in_offers_every_workspace_and_greys_none() {
    let doc = include_str!("../../docs/direction/Workbench_UX_Design.md");
    assert!(
        doc.contains("none is greyed"),
        "the design no longer says that none is greyed"
    );
    let stale = "greys the ones that do not show the asset";
    assert!(!doc.contains(stale), "the design still says `{stale}`");
}

/// Clicks the widget named `name` and settles.
fn click_named(shell: &mut Shell, name: &str) {
    let at = super::the_project_screens_on_screen::centre_of(shell, name);
    shell.click(at);
    shell.settled();
}

#[test]
fn the_assets_table_reads_the_quick_look_key_as_its_own_under_the_pointer() {
    use crate::ui::commands::Action;
    use crate::ui::shortcuts;
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    shell.state.active_workspace = Workspace::Project;
    shell.settled();
    // The click selects the script and leaves the pointer over the table.
    click_named(&mut shell, "client.lua");
    assert!(!shell.state.workbench.quick_look);
    let press = |shell: &mut Shell, key| {
        shell.draw(vec![super::frames_of_the_shell::key(key)]);
        shell.settled();
    };
    press(&mut shell, egui::Key::Space);
    assert!(
        shell.state.workbench.quick_look,
        "Space over the Assets table does not look at the selected asset"
    );

    // F frames the selection in the viewport, and the table reads it as Quick look once that command is given it too.
    let none = shortcuts::ShortcutMods::none();
    let shared = shortcuts::set_binding(
        &mut shell.state.prefs,
        Action::QuickLook,
        none,
        egui::Key::F,
    );
    assert!(shared.is_ok(), "{shared:?}");
    press(&mut shell, egui::Key::F);
    assert!(
        !shell.state.workbench.quick_look,
        "the table left a key it shares with the viewport to the viewport"
    );
}

#[test]
fn a_filter_that_matches_nothing_says_so_and_clear_brings_the_assets_back() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    shell.state.active_workspace = Workspace::Project;
    shell.settled();
    assert!(shows(&shell, "client.lua"), "the table lists the script");

    click_named(&mut shell, "Filter assets");
    shell.draw(vec![egui::Event::Text("zzzz".to_string())]);
    shell.settled();
    assert!(
        shows(&shell, "No asset matches the filter"),
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(!shows(&shell, "client.lua"), "and no row is listed");

    click_named(&mut shell, "Clear filters");
    assert!(shows(&shell, "client.lua"), "Clear lists every asset again");
    assert!(!shows(&shell, "No asset matches"));
    let clear = (shell.described.iter())
        .find(|w| w.name.as_deref() == Some("Clear filters"))
        .expect("the button stays");
    assert!(clear.disabled, "and is greyed while no filter is on");
}

#[test]
fn problems_wrap_their_messages_and_every_message_starts_at_one_place() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    shell.state.active_workspace = Workspace::Project;
    let size = (700, 600);
    until_sized(&mut shell, size, "the rows of Problems", |s| {
        s.state.workbench.findings().len() >= 2 && says(s, "missing_hat.ydd is missing")
    });

    let texts: Vec<String> = shell
        .state
        .workbench
        .findings()
        .iter()
        .map(|f| f.text.clone())
        .collect();
    // The label holds the message with the breaks its layout was given, so what is compared is the words.
    let bare = |text: &str| text.split_whitespace().collect::<String>();
    let at = |text: &str| {
        (shell.described.iter())
            .find(|w| {
                w.role == egui::accesskit::Role::Label
                    && w.name
                        .as_deref()
                        .is_some_and(|name| bare(name) == bare(text))
            })
            .map(|w| w.bounds)
            .unwrap_or_else(|| panic!("no message `{text}`"))
    };
    let rects: Vec<egui::Rect> = texts.iter().map(|text| at(text)).collect();
    // ⚠ Messages are drawn whole: a long one takes more lines and is never cut to one.
    let longest = texts
        .iter()
        .enumerate()
        .max_by_key(|(_, t)| t.len())
        .map(|(i, _)| i)
        .unwrap();
    assert!(
        rects[longest].height() > 24.0,
        "the longest message wraps: {:?}",
        rects[longest]
    );
    // ⚠ One alignment: whatever buttons a row has, its message starts where every other one does.
    let first = rects[0].min.x;
    assert!(
        rects.iter().all(|r| (r.min.x - first).abs() < 1.0),
        "the messages start at {:?}",
        rects.iter().map(|r| r.min.x).collect::<Vec<_>>()
    );
    // And none runs past the panel it is in.
    // The tabs of the panel's strip span it, so the last one's right edge is the panel's.
    let panel_right = (shell.described.iter())
        .filter(|w| {
            w.role == egui::accesskit::Role::Button
                && matches!(
                    w.name.as_deref(),
                    Some("Problems" | "Build" | "History" | "Jobs" | "Log")
                )
        })
        .filter(|w| w.bounds.min.y > 200.0 && w.bounds.min.y < size.1 as f32 * 0.7)
        .map(|w| w.bounds.max.x)
        .fold(0.0, f32::max);
    assert!(
        rects.iter().all(|r| r.max.x <= panel_right + 1.0),
        "{rects:?} against {panel_right}"
    );

    // ⚠ A path in a message breaks after a separator: every name in every message is on one line whole.
    let folder = or_project::project_folder(&shell.state.project_path.clone().expect("a project"));
    let long = folder.join("stream/the_longest_folder_name_there_is_for_this_test/another_long_name_for_a_missing_file.ydd");
    shell.state.add_files_to_project(vec![long]);
    shell.state.workbench.want_scan();
    // Tall, so every row of Problems is in view.
    let narrow = (480, 1600);
    until_sized(&mut shell, narrow, "the long message", |s| {
        says(s, "another_long_name_for_a_missing_file.ydd is missing")
    });
    let shapes = shell.draw_sized(narrow, Vec::new());
    let findings: Vec<String> = (shell.state.workbench.findings().iter())
        .map(|f| f.text.clone())
        .collect();
    for text in findings
        .iter()
        .filter(|text| text.contains(" is missing: "))
    {
        let first_word = text.split(" ").next().unwrap_or_default();
        let lines = rows_painted(&shapes, first_word);
        for name in text.split([' ', '\\', '/']).filter(|name| name.len() > 3) {
            assert!(
                lines.iter().any(|line| line.contains(name)),
                "`{name}` was cut: {lines:?}"
            );
        }
    }
}

#[test]
fn linked_content_is_counted_like_any_and_the_banner_alone_says_it_is_linked() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let (mut shell, _owner) = super::the_wardrobe_on_screen::a_shell_with_a_linked_pack(dir.path());
    shell.state.active_workspace = Workspace::Project;
    until(&mut shell, "the banner", |s| {
        shows(s, "Linked from brand.openrage-proj")
    });

    assert!(
        shows(&shell, "Clothing  ·  1 garment"),
        "a linked content says what it holds as any content does: {:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    let says_link = (shell.described.iter())
        .filter(|w| w.role == egui::accesskit::Role::Label)
        .filter(|w| {
            w.name
                .as_deref()
                .is_some_and(|n| n.to_lowercase().contains("link"))
        })
        .count();
    assert_eq!(
        says_link,
        1,
        "linked is said once, by the banner: {:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
}

#[test]
fn a_project_with_nothing_in_it_offers_one_next_action_and_says_where_the_other_doors_are() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut draft = crate::state::front_door::NewProject::new(
        crate::state::front_door::Template::Files,
        dir.path().to_path_buf(),
    );
    draft.set_name("bare".into());
    let file = draft.create().expect("a project").file;
    let mut shell = Shell::new(ThemeChoice::Dark);
    shell.state.load_project_from(file);
    shell.state.active_workspace = Workspace::Project;
    until(&mut shell, "the landing", |s| {
        s.state.workbench.is_scanned() && shows(s, "holds no files yet")
    });

    // ⚠ One filled door and two quiet ones, each with a line under it that says what it takes, and no menu named: the doors are buttons.
    let mut landing_buttons: Vec<_> = (shell.described.iter())
        .filter(|w| w.role == egui::accesskit::Role::Button)
        .filter_map(|w| w.name.as_deref())
        .filter(|n| ["Add files…", "Add a clothing pack…", "Import game files…"].contains(n))
        .collect();
    landing_buttons.sort_unstable();
    assert_eq!(
        landing_buttons,
        ["Add a clothing pack…", "Add files…", "Import game files…"],
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(
        shows(&shell, "You can also drop files or folders here"),
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(
        !shows(&shell, "Project menu"),
        "no sentence sends a person to a menu for a door that is a button: {:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(!shell.state.has_clothing());
}

/// Clicks the button named `button` that is on the row of Problems whose link is named `link`.
fn click_on_the_row_of(shell: &mut Shell, link: &str, button: &str) {
    let row = shell
        .described
        .iter()
        .find(|w| w.role == egui::accesskit::Role::Link && w.name.as_deref() == Some(link))
        .unwrap_or_else(|| panic!("no row of Problems about `{link}`"))
        .bounds
        .center()
        .y;
    let at = (shell.described.iter())
        .find(|w| {
            w.role == egui::accesskit::Role::Button
                && w.name.as_deref() == Some(button)
                && (w.bounds.center().y - row).abs() < 4.0
        })
        .unwrap_or_else(|| panic!("no `{button}` on the row of `{link}`"))
        .bounds
        .center();
    shell.click(at);
    shell.settled();
}

#[test]
fn a_missing_source_member_is_a_row_of_problems_and_locate_points_it_at_the_folder_it_moved_to() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    let folder = or_project::project_folder(&shell.state.project_path.clone().expect("a project"));
    let source = a_source_of_one_png(&folder, "logo");
    assert_eq!(
        shell.state.add_files_to_project(vec![source.clone()]).added,
        1
    );
    shell.state.workbench.want_scan();
    until(&mut shell, "the source's row", |s| {
        s.state
            .workbench
            .rows()
            .iter()
            .any(|row| row.name == "logo")
    });

    // The folder is moved in a file manager: the member is gone, and the walk finds the folder where it is now.
    let moved = folder.join("moved").join("logo.textures.src");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&source, &moved).unwrap();
    shell.state.workbench.want_scan();
    shell.state.active_workspace = Workspace::Project;
    until(&mut shell, "the row of Problems", |s| {
        says(s, "logo.textures.src is missing")
    });
    let finding = (shell.state.workbench.findings().iter())
        .find(|f| f.text.contains("logo.textures.src is missing"))
        .expect("the missing member is a finding");
    assert_eq!(finding.severity, crate::state::workbench::Severity::Error);
    let ways: Vec<_> = finding.fixes.iter().map(|fix| fix.label()).collect();
    assert_eq!(ways, ["Locate…", "Remove from project"]);

    // Locate asks for a folder, and what it is answered with is where the member is now.
    crate::platform::file_dialogs::answer_next(moved.clone());
    click_on_the_row_of(&mut shell, "logo.textures.src", "Locate…");
    assert_eq!(
        crate::platform::file_dialogs::last_dialog().as_deref(),
        Some("Locate source folder"),
        "a folder was asked for"
    );
    until(&mut shell, "the repair", |s| {
        !says(s, "logo.textures.src is missing")
    });
    assert!(
        (shell.state.project_carried.members.iter()).any(|m| m.path == moved),
        "the member is the folder it moved to"
    );
    let rows = (shell.state.workbench.rows().iter())
        .filter(|row| row.name == "logo")
        .count();
    assert_eq!(rows, 1, "and the table lists the source once");
}

/// Copies the folder `from` to `to`, as a file manager does.
fn copy_folder(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_folder(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Whether some finding of the project offers a new id.
fn offers_a_new_id(shell: &Shell) -> bool {
    (shell.state.workbench.findings().iter()).any(|f| f.fixes.iter().any(|x| x.label() == "New id"))
}

#[test]
fn a_source_copied_by_hand_is_an_error_on_each_folder_and_new_id_on_the_copy_leaves_the_original_alone(
) {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    let folder = or_project::project_folder(&shell.state.project_path.clone().expect("a project"));
    let original = a_source_of_one_png(&folder, "logo");
    // ⚠ A file manager names a copy `logo - Copy`, and the table lists `logo copy` before `logo.`: the copy is the first row.
    let copy = folder.join("sources").join("logo copy.textures.src");
    copy_folder(&original, &copy);
    shell.state.workbench.want_scan();
    shell.state.active_workspace = Workspace::Project;
    until(&mut shell, "the rows of Problems", |s| {
        offers_a_new_id(s) && says(s, "logo copy.textures.src")
    });
    let id_of = |dir: &std::path::Path| {
        or_source::asset::Source::open(dir)
            .expect("a source")
            .manifest
            .id
    };
    let id = id_of(&original);
    assert_eq!(id, id_of(&copy), "a copy keeps the id");

    // Nothing in either folder says which is the copy, so each row can be given the new id and the message calls neither one.
    let buttons = (shell.described.iter())
        .filter(|w| w.role == egui::accesskit::Role::Button && w.name.as_deref() == Some("New id"))
        .count();
    assert_eq!(
        buttons,
        2,
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(
        says(
            &shell,
            "logo copy.textures.src and logo.textures.src come from the same file"
        ),
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(!says(&shell, "is a copy of"));

    click_on_the_row_of(&mut shell, "logo copy.textures.src", "New id");

    until(&mut shell, "the problem to go", |s| !offers_a_new_id(s));
    assert_eq!(id_of(&original), id, "the original keeps its id");
    assert_ne!(id_of(&copy), id, "the copy has an id of its own");
}

/// The rows of the text painted in `shapes` that holds `containing`, as the layout broke them.
fn rows_painted(shapes: &[egui::epaint::ClippedShape], containing: &str) -> Vec<String> {
    fn walk(shape: &egui::Shape, containing: &str, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) if text.galley.text().contains(containing) => {
                out.extend(text.galley.rows.iter().map(|placed| placed.row.text()));
            }
            egui::Shape::Vec(inner) => inner.iter().for_each(|s| walk(s, containing, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes
        .iter()
        .for_each(|clipped| walk(&clipped.shape, containing, &mut out));
    out
}

/// Opens the folded section `name` of a panel, in a window of `size` points.
fn open_section(shell: &mut Shell, size: (u32, u32), name: &str) {
    let at = super::the_project_screens_on_screen::centre_of(shell, name);
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    shell.draw_sized(size, vec![egui::Event::PointerMoved(at), press(true)]);
    shell.draw_sized(size, vec![press(false)]);
    shell.settled_sized(size);
}

/// The lines of a label's text, which is the whole text of the widget with the breaks its layout was given.
fn lines_of(shell: &Shell, containing: &str) -> Vec<String> {
    let name = (shell.described.iter())
        .filter_map(|w| w.name.clone())
        .find(|n| n.contains(containing))
        .unwrap_or_else(|| {
            panic!(
                "nothing says `{containing}`: {:?}",
                super::the_project_screens_on_screen::names(shell)
            )
        });
    name.lines().map(str::to_string).collect()
}

#[test]
fn a_narrow_window_breaks_a_path_after_a_separator_and_never_inside_a_name() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    shell.state.active_workspace = Workspace::Project;
    // The project's own properties say where its file is, under Project file, which holds a whole path of this machine.
    shell.state.select_asset(None);
    // ⚠ Sixty points taller than it was: the row of tabs a project has takes its height from the panel, and the fold must be on screen to be opened.
    until_sized(&mut shell, (480, 660), "Properties", |s| {
        shows(s, "Project file")
    });
    open_section(&mut shell, (480, 660), "Project file");

    // Properties says where the file is, in as many lines as the panel's width asks for.
    let lines = lines_of(&shell, ".openrage-proj");
    assert!(
        lines.len() > 1,
        "a long path in a narrow panel takes several lines: {lines:?}"
    );
    for line in &lines[..lines.len() - 1] {
        assert!(
            line.ends_with(['\\', '/']),
            "a line of a path ends where a folder does, never inside a name: {lines:?}"
        );
    }
    assert!(lines
        .last()
        .is_some_and(|last| last.contains(".openrage-proj")));
}

#[test]
fn a_window_too_narrow_for_the_columns_says_which_are_hidden() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    shell.state.active_workspace = Workspace::Project;
    until_sized(&mut shell, (700, 600), "the cue", |s| {
        shows(s, "columns hidden")
    });
    let said = lines_of(&shell, "columns hidden").join(" ");
    assert!(
        said.starts_with(|c: char| c.is_ascii_digit()) && said.ends_with("hidden"),
        "the phrase is one piece and counts them: {said}"
    );

    // At a width where they all fit there is nothing to say.
    let wide_dir = tempfile::tempdir().unwrap();
    let mut wide = a_project_shell(wide_dir.path(), ThemeChoice::Dark);
    wide.state.active_workspace = Workspace::Project;
    until_sized(&mut wide, (1600, 900), "the table", |s| {
        shows(s, "client.lua")
    });
    assert!(
        !shows(&wide, "columns hidden"),
        "{:?}",
        super::the_project_screens_on_screen::names(&wide)
    );
}

#[test]
fn a_project_with_no_asset_never_shows_the_filter_state_of_the_project_before_it() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    shell.state.active_workspace = Workspace::Project;
    until_sized(&mut shell, (700, 600), "the narrow table", |s| {
        shows(s, "columns hidden")
    });

    let mut draft = crate::state::front_door::NewProject::new(
        crate::state::front_door::Template::Files,
        dir.path().to_path_buf(),
    );
    draft.set_name("bare".into());
    let bare = draft.create().expect("a project").file;
    // So that leaving the project asks nothing about its unsaved files.
    assert!(shell.state.save_active_project(|_| None));
    shell.state.load_project_from(bare);
    until_sized(&mut shell, (700, 600), "the landing", |s| {
        s.state.workbench.is_scanned() && shows(s, "holds no files yet")
    });

    assert!(
        shows(&shell, "0 of 0 assets"),
        "{:?}",
        super::the_project_screens_on_screen::names(&shell)
    );
    assert!(
        !shows(&shell, "columns hidden"),
        "there is no table whose columns could be hidden"
    );
}

#[test]
fn a_quick_look_in_a_window_too_small_to_dock_in_draws_over_nothing() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    let key = (shell.state.workbench.rows().iter())
        .find(|row| row.name == "client.lua")
        .expect("the script's row")
        .key
        .clone();
    shell.state.active_workspace = Workspace::Project;
    shell.state.select_asset(Some(key));
    shell.state.workbench.quick_look = true;
    until_sized(&mut shell, (480, 600), "the quick look", |s| {
        shows(s, "Quick look: client.lua")
    });

    let title = (shell.described.iter())
        .find(|w| {
            w.name
                .as_deref()
                .is_some_and(|n| n.starts_with("Quick look:"))
        })
        .expect("the title")
        .bounds;
    let under: Vec<_> = (shell.described.iter())
        .filter(|w| w.role == egui::accesskit::Role::Label)
        .filter(|w| w.bounds.intersects(title))
        .filter_map(|w| w.name.clone())
        .filter(|name| !name.starts_with("Quick look:"))
        .collect();
    assert!(
        under.is_empty(),
        "the quick look's title is drawn over {under:?}"
    );
}

/// Where Properties puts the value of the fact called `name`, and where it puts the name.
fn fact_at(shell: &Shell, name: &str, value: &str) -> (egui::Rect, egui::Rect) {
    let find = |text: &str| {
        (shell.described.iter())
            .find(|w| w.role == egui::accesskit::Role::Label && w.name.as_deref() == Some(text))
            .unwrap_or_else(|| {
                panic!(
                    "no label `{text}`: {:?}",
                    super::the_project_screens_on_screen::names(shell)
                )
            })
            .bounds
    };
    (find(name), find(value))
}

#[test]
fn properties_puts_a_value_under_its_name_where_two_columns_would_leave_it_a_few_letters() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    let key = (shell.state.workbench.rows().iter())
        .find(|row| row.name == "client.lua")
        .expect("the script's row")
        .key
        .clone();
    shell.state.active_workspace = Workspace::Project;
    shell.state.select_asset(Some(key));

    // Wide, the name is beside its value; the size is among the facts folded under More.
    until_sized(&mut shell, (1600, 900), "Properties", |s| shows(s, "More"));
    open_section(&mut shell, (1600, 900), "More");
    until_sized(&mut shell, (1600, 900), "Properties", |s| shows(s, "14 B"));
    let (name, value) = fact_at(&shell, "Size", "14 B");
    assert!(value.min.x >= name.max.x, "beside: {name:?} {value:?}");

    // Narrow, it is under it, with the whole width of the panel.
    until_sized(&mut shell, (900, 700), "Properties", |s| shows(s, "14 B"));
    let (name, value) = fact_at(&shell, "Size", "14 B");
    assert!(
        value.min.y >= name.max.y - 1.0 && (value.min.x - name.min.x).abs() < 1.0,
        "under: {name:?} {value:?}"
    );
}

#[test]
fn a_content_whose_row_and_problem_count_do_not_fit_one_line_puts_the_count_on_the_next() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    shell.state.active_workspace = Workspace::Project;
    until_sized(&mut shell, (480, 600), "the count", |s| {
        s.state.workbench.is_scanned() && says(s, "1 error")
    });

    let right_of_the_panel = (shell.described.iter())
        .find(|w| w.name.as_deref() == Some("Filter assets"))
        .expect("the filter, which starts the Assets panel")
        .bounds
        .min
        .x;
    let tags: Vec<egui::Rect> = (shell.described.iter())
        .filter(|w| {
            w.name.as_deref().map(str::trim_start).is_some_and(|n| {
                n.starts_with("● ") && (n.contains("error") || n.contains("warning"))
            })
        })
        .map(|w| w.bounds)
        .collect();
    assert!(
        !tags.is_empty(),
        "the Content and Builds panels count their errors and warnings"
    );
    for tag in tags {
        assert!(
            tag.max.x <= right_of_the_panel,
            "{tag:?} is cut by the panel that ends at {right_of_the_panel}"
        );
    }
}
