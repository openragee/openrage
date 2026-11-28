//! A weapon of the owner's install, with its own model and textures, is imported into a weapon pack and its assembled view is read.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass: the clean checkout holds no game file,
//! and the other suites of the Weapons screen draw weapons of documents their file writes.
//!
//! ⚠ The files are copied out of the install into a scratch folder and imported from there, so the install is read and never
//! written, and nothing of the game stays in the repository.

use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::{build_mounted_game, DlcCutoff, GtaInstall};

use super::the_front_door_on_screen::a_project;
use super::the_project_screens_on_screen::until;
use super::the_weapons_screen_on_screen::write;
use crate::state::front_door::Template;
use crate::ui::dock::Workspace;

const WHAT: &str = "a_weapon_of_the_install_is_imported_and_its_assembled_view_is_read";

/// The weapon the test draws, with the files its metadata names.
const WEAPON: &str = "WEAPON_PISTOL";
const MODEL: &str = "w_pi_pistol";

#[test]
fn a_weapon_of_the_install_is_imported_and_its_assembled_view_is_read() {
    let _serial = crate::ui::theme::serial();
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format_args!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let Some(exe) = Game::detect_from_root(&root).and_then(|game| game.find_executable(&root))
    else {
        skip_install(WHAT, format_args!("the install has no executable"));
        return;
    };
    let Ok(keys) = extract_keys(&exe, &|_| {}) else {
        skip_install(WHAT, format_args!("the keys could not be read"));
        return;
    };
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    let Some(install) = GtaInstall::from_path(&root) else {
        skip_install(WHAT, format_args!("the root is no install"));
        return;
    };
    let Ok(game) = build_mounted_game(&install, crypto.clone(), DlcCutoff::All) else {
        skip_install(WHAT, format_args!("the install could not be mounted"));
        return;
    };

    // The weapon file that defines the pistol, its model and its textures, by the names the install gives them.
    let wanted = [format!("{MODEL}.ydr"), format!("{MODEL}.ytd")];
    let (mut metas, mut files) = (Vec::new(), Vec::new());
    game.for_each_file(|logical, _, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if leaf == "weapons.meta" {
            metas.push(logical.to_string());
        } else if wanted.contains(&leaf)
            && !files
                .iter()
                .any(|(name, _): &(String, String)| *name == leaf)
        {
            files.push((leaf, logical.to_string()));
        }
    });
    let meta = metas
        .iter()
        .filter_map(|logical| game.vfs.open(logical))
        .find(|bytes| String::from_utf8_lossy(bytes).contains(&format!("<Name>{WEAPON}</Name>")))
        .expect("a weapons.meta of the install defines the pistol");
    assert_eq!(
        files.len(),
        2,
        "the install holds the pistol's model and textures: {files:?}"
    );

    let dir = tempfile::tempdir().unwrap();
    let mut dropped = vec![write(&dir.path().join("drop").join("weapons.meta"), &meta)];
    for (name, logical) in &files {
        let bytes = game.vfs.open(logical).expect("the file reads");
        dropped.push(write(&dir.path().join("drop").join(name), &bytes));
    }

    let file = a_project(dir.path(), "arsenal", Template::Weapons);
    let mut shell = super::frames_of_the_shell::Shell::new(crate::state::ThemeChoice::Dark);
    shell.state.load_project_from(file);
    shell.state.add_weapons(dropped);
    shell.state.active_workspace = Workspace::Weapons;
    shell.settled();
    until(&mut shell, "the pistol to be read", |s| {
        s.state
            .weapons_pack()
            .and_then(|p| p.read.as_ref().ok())
            .is_some_and(|read| read.model.weapons.iter().any(|w| w.name == WEAPON))
    });

    let (module, id) = {
        let pack = shell.state.weapons_pack().expect("a pack");
        let read = pack.read.as_ref().expect("a pack that reads");
        let weapon = read
            .model
            .weapons
            .iter()
            .find(|w| w.name == WEAPON)
            .expect("the pistol");
        let kinds: Vec<_> = weapon
            .sources
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    read.facts.source(path).map(|s| s.kind.clone()),
                )
            })
            .collect();
        assert!(
            kinds.iter().any(|(_, k)| k.as_deref() == Some("drawable"))
                && kinds.iter().any(|(_, k)| k.as_deref() == Some("textures")),
            "the pistol is given its model and its textures by the name its metadata states: {kinds:?}"
        );
        (pack.module.clone(), weapon.id.clone())
    };
    shell.state.select_weapon(&module, Some(id));
    shell.settled();

    // The assembled view reads the model and the textures the pack holds, as the preview panel asks for them.
    let request = shell
        .state
        .weapon_scene_request()
        .expect("a weapon with a model has a scene to read");
    shell.state.workbench.weapons.scene.want(request, crypto);
    let started = std::time::Instant::now();
    while shell.state.workbench.weapons.scene.is_reading() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(120),
            "the scene was not read in two minutes"
        );
        shell.state.workbench.weapons.scene.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let work = &shell.state.workbench.weapons.scene;
    assert_eq!(work.error, None, "the scene reads");
    let scene = work.scene.as_ref().expect("a scene was read");
    assert!(!scene.parts.is_empty(), "the model has a mesh to draw");
    assert!(
        scene.parts.iter().all(|part| part.diffuse.is_some()),
        "the textures of the pack reach the parts"
    );
}
