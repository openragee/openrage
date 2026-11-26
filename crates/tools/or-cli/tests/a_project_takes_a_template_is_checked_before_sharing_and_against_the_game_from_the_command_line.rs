//! `openrage-cli project` does what the Project workspace's Apply a template…, Check before sharing… and Check against the game do, with no window:
//! `apply-template` adds what a template states and the project lacks, `check-sharing` exits 1 where the folder holds what is not yours to send,
//! and `check-original` exits 1 where the game's file a source was taken from is another file now.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_project::Project;
use or_source::game_import::{decompile, Facts};
use or_source::sample;

fn project(args: &[&str]) -> ExitCode {
    let mut all = vec!["project".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

fn write(path: &Path, body: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A project `name` in a folder of its own with one file in its Files content.
fn a_project(dir: &Path, name: &str) -> PathBuf {
    let folder = dir.join(name);
    let script = folder.join("client.lua");
    write(&script, b"print('hi')");
    assert_eq!(
        project(&["create", text(&folder), "--name", name, text(&script)]),
        ExitCode::SUCCESS
    );
    folder.join(format!("{name}.openrage-proj"))
}

#[test]
fn a_template_is_applied_to_a_project_that_exists_and_adds_only_what_it_lacks() {
    let dir = tempfile::tempdir().unwrap();
    let made = a_project(dir.path(), "police");
    or_project::project::edit_container(&made, |container| {
        container
            .modules
            .push(or_project::Module::clothing("uniforms", "modules/uniforms"));
        container.outputs[0].take.push("uniforms".into());
        Ok(())
    })
    .unwrap();
    let template = dir.path().join("police.openrage-template");
    assert_eq!(
        project(&["template", text(&made), text(&template)]),
        ExitCode::SUCCESS
    );

    let harbor = a_project(dir.path(), "harbor");
    let before = Project::load(&harbor).unwrap().container;
    assert!(before.module("uniforms").is_none());

    assert_eq!(
        project(&["apply-template", text(&harbor), text(&template)]),
        ExitCode::SUCCESS
    );
    let after = Project::load(&harbor).unwrap().container;
    assert!(
        after.module("uniforms").is_some(),
        "the clothing the template states is added"
    );
    assert!(
        harbor
            .parent()
            .unwrap()
            .join("modules/uniforms/wardrobe.json")
            .is_file(),
        "and its folder is started with an empty pack"
    );
    assert_eq!(
        after.module("files"),
        before.module("files"),
        "what the project held is as it was"
    );

    assert_eq!(
        project(&["apply-template", text(&harbor), text(&template)]),
        ExitCode::FAILURE,
        "a template that adds nothing is refused, as the window refuses it"
    );
    assert_eq!(
        Project::load(&harbor).unwrap().container,
        after,
        "and writes nothing"
    );
}

#[test]
fn a_folder_with_a_path_of_this_machine_and_a_source_from_the_game_fails_the_sharing_check() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path(), "harbor");
    let folder = file.parent().unwrap().to_path_buf();
    assert_eq!(
        project(&["check-sharing", text(&file)]),
        ExitCode::SUCCESS,
        "a fresh project has nothing to take out"
    );

    let game = Facts {
        from_game: true,
        virtual_path: Some("x64/pack.rpf/road.ytd".into()),
        ..Facts::default()
    };
    std::fs::create_dir_all(folder.join("sources")).unwrap();
    decompile(
        &sample::textures_of("road"),
        "road.ytd",
        &folder.join("sources"),
        &game,
    )
    .expect("a source taken from the game");
    assert_eq!(
        project(&["check-sharing", text(&file)]),
        ExitCode::from(1),
        "a source taken from the game is the game's own file"
    );
    assert_eq!(
        project(&["check-sharing", text(&file), "--json"]),
        ExitCode::from(1)
    );
    assert_eq!(
        project(&["check-sharing"]),
        ExitCode::from(2),
        "it needs a project"
    );
}

/// ⚠ A manifest a maps content keeps is the game's file whole, with no `asset.json` to say so: where it lies is what says it.
#[test]
fn a_manifest_of_the_game_a_maps_content_keeps_fails_the_sharing_check_and_is_ignored_by_git() {
    use or_maps::sample::{manifest_file, Stated};

    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path(), "harbor");
    let folder = file.parent().unwrap().to_path_buf();
    assert_eq!(
        project(&["check-sharing", text(&file)]),
        ExitCode::SUCCESS,
        "the project holds nothing of the game yet"
    );

    let stated = Stated {
        map: "harbour",
        flags: 1,
        types: &["harbour_types"],
    };
    let kept = or_maps::manifest::keep(&folder.join("maps"), &manifest_file(&[stated])).unwrap();
    let inside = format!(
        "maps/{}/{}",
        or_maps::manifest::KEPT,
        kept.file_name().unwrap().to_string_lossy()
    );
    let concerns = or_content::sharing::concerns(&file, &[], &[], false);
    assert!(
        (concerns.iter())
            .any(|c| c.kind == or_content::sharing::Kind::GameContent && c.text.contains(&inside)),
        "the kept manifest is named as the game's: {concerns:?}"
    );
    assert_eq!(
        project(&["check-sharing", text(&file)]),
        ExitCode::from(1),
        "a pre-push hook that runs the verb is stopped"
    );

    or_source::git::update(&folder).unwrap();
    let ignore = std::fs::read_to_string(folder.join(".gitignore")).unwrap();
    assert!(
        ignore
            .lines()
            .any(|line| line == or_source::git::KEPT_MANIFESTS),
        "{ignore}"
    );
    assert_eq!(
        or_source::git::KEPT_MANIFESTS,
        format!("**/{}/*.ymf", or_maps::manifest::KEPT),
        "the line is about the folder a maps content keeps them in"
    );
}

#[test]
fn the_private_folders_an_edit_leaves_are_listed_and_do_not_fail_the_sharing_check() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path(), "harbor");
    let folder = file.parent().unwrap().to_path_buf();
    or_project::project::edit_container(&file, |container| {
        container
            .modules
            .push(or_project::Module::clothing("uniforms", "modules/uniforms"));
        Ok(())
    })
    .unwrap();
    let pack = folder.join("modules/uniforms");
    std::fs::create_dir_all(&pack).unwrap();
    let wardrobe = or_wardrobe::Wardrobe::new("uniforms");
    wardrobe.write(&pack).unwrap();
    wardrobe.write(&pack).unwrap();
    std::fs::create_dir_all(folder.join(".openrage")).unwrap();
    let concerns = or_content::sharing::concerns(&file, &[], std::slice::from_ref(&pack), false);
    assert_eq!(
        concerns
            .iter()
            .filter(|c| c.kind == or_content::sharing::Kind::PrivateFolder)
            .count(),
        2,
        "the pack's earlier versions and the project's own folder are listed: {concerns:?}"
    );

    assert_eq!(
        project(&["check-sharing", text(&file)]),
        ExitCode::SUCCESS,
        "Git never sees these folders, so a pre-push hook running the verb is not stopped by them"
    );
    assert_eq!(
        project(&["check-sharing", text(&file), "--json"]),
        ExitCode::SUCCESS
    );
}

#[test]
fn a_source_is_checked_against_the_install_and_the_exit_code_says_when_the_game_changed_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path(), "harbor");
    let folder = file.parent().unwrap().to_path_buf();
    let original = sample::textures_of("road");
    let game = Facts {
        from_game: true,
        virtual_path: Some("x64/mods/road.ytd".into()),
        ..Facts::default()
    };
    std::fs::create_dir_all(folder.join("sources")).unwrap();
    decompile(&original, "road.ytd", &folder.join("sources"), &game).expect("a source");
    let install = dir.path().join("install");

    write(&install.join("x64/mods/road.ytd"), &original);
    assert_eq!(
        project(&["check-original", text(&file), "--game", text(&install)]),
        ExitCode::SUCCESS,
        "the game's file is the one the source was taken from"
    );

    write(
        &install.join("x64/mods/road.ytd"),
        &sample::textures_of("another"),
    );
    assert_eq!(
        project(&["check-original", text(&file), "--game", text(&install)]),
        ExitCode::from(1),
        "the game's file is another file now"
    );

    std::fs::remove_file(install.join("x64/mods/road.ytd")).unwrap();
    assert_eq!(
        project(&["check-original", text(&file), "--game", text(&install)]),
        ExitCode::from(1),
        "the game removed it"
    );
    assert_eq!(
        project(&["check-original", text(&file)]),
        ExitCode::from(2),
        "it needs the install to read"
    );
}
