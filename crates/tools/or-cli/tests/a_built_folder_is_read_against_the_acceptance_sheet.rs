//! The recipe of `docs/guides/In_Game_Acceptance.md` runs over a synthetic map and type file: its `openrage-cli project`
//! commands, read from the guide, make a project, import both as the game's, build a FiveM resource after an edit of each
//! source, and name the lines of the sheet about what the build wrote.
//!
//! ⚠ The files are `or_maps::sample`'s and hold no game content, so this says the commands run and nothing of what the game does.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_io::decl::File;
use or_io::meta::typed::{CMapData, CMapTypes};
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_project::acceptance::{kind_of, kinds_in, knows, Place, Seen, Sheet};
use or_project::Target;

const CHILD_TEST: &str = "the_child_that_runs_one_command";
const CHILD_ENV: &str = "OPENRAGE_TEST_ONE_COMMAND";
const BEGIN: &str = "<<begin>>";
const END: &str = "<<end>>";
const SEPARATOR: char = '\u{1f}';
const GUIDE: &str = "docs/guides/In_Game_Acceptance.md";
/// The heading of the guide's recipe, which its anchor is made of.
const RECIPE: &str = "## Make the sample";

/// Run by `cli` in a child process, so that what a command prints can be read.
#[test]
fn the_child_that_runs_one_command() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args: Vec<String> = args
        .to_string_lossy()
        .split(SEPARATOR)
        .map(str::to_string)
        .collect();
    let mut out = std::io::stdout();
    let _ = write!(out, "{BEGIN}");
    let _ = out.flush();
    let code = or_cli::run(&args);
    let _ = out.flush();
    let _ = write!(out, "{END}{}", u8::from(code == ExitCode::SUCCESS));
    let _ = out.flush();
}

/// Whether `openrage-cli <args>` succeeded, and what it printed.
fn cli(args: &[&str]) -> (bool, String) {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("{args:?}: no start marker: {text}"));
    let (printed, tail) = rest.split_once(END).unwrap_or((rest, ""));
    (tail.starts_with('1'), printed.to_string())
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

/// The `openrage-cli project` commands of the guide's recipe, in its order, as the guide writes each: a verb and its arguments.
fn the_recipe() -> Vec<Vec<String>> {
    let root = or_test_support::openrage_root().expect("the tests run inside the tree");
    let guide = std::fs::read_to_string(root.join(GUIDE)).expect("the guide is in the tree");
    let recipe: Vec<&str> = (guide.lines())
        .skip_while(|line| *line != RECIPE)
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .collect();
    (recipe.iter())
        .filter_map(|line| line.trim().strip_prefix("openrage-cli project "))
        .map(|command| command.split_whitespace().map(str::to_string).collect())
        .collect()
}

/// Run the command of the recipe whose verb is `verb`, with the guide's placeholders filled for `work`, and say what it printed.
fn run_step(verb: &str, work: &Path) -> String {
    let recipe = the_recipe();
    let steps: Vec<&Vec<String>> = (recipe.iter())
        .filter(|command| command[0] == verb)
        .collect();
    let [step] = steps.as_slice() else {
        panic!("the recipe of {GUIDE} has `project {verb}` once: {recipe:?}");
    };
    let filled: Vec<String> = (step.iter())
        .map(|word| {
            word.replace("<scratch>", text(work))
                .replace("<map>", "harbour")
                .replace("<types>", "harbour_types")
        })
        .collect();
    let mut args = vec!["project"];
    args.extend(filled.iter().map(String::as_str));
    let (ran, said) = cli(&args);
    assert!(ran, "{args:?}: {said}");
    said
}

/// Replace the one `from` of the source `name` of the project in `folder` with `to`.
fn edit(folder: &Path, name: &str, from: &str, to: &str) {
    let xml = folder
        .join("sources")
        .join(format!("{name}.metadata.src"))
        .join("metadata.xml");
    let before = std::fs::read_to_string(&xml).unwrap();
    assert_eq!(before.matches(from).count(), 1, "{from} in {before}");
    std::fs::write(&xml, before.replace(from, to)).unwrap();
}

/// The project of the recipe, with its map and type file imported as the game's and each edited as the recipe says.
fn the_sample(work: &Path) -> PathBuf {
    let game = work.join("game");
    std::fs::create_dir_all(&game).unwrap();
    let map = map_file(
        "harbour",
        &[Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1)],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    );
    let types = types_file(
        "harbour_types",
        &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])],
    );
    std::fs::write(game.join("harbour.ymap"), map).unwrap();
    std::fs::write(game.join("harbour_types.ytyp"), types).unwrap();

    let folder = work.join("acceptance");
    let file = folder.join("acceptance.openrage-proj");
    run_step("create", work);
    assert!(file.is_file(), "the recipe makes {}", file.display());
    run_step("import", work);

    // The object moved two metres, a copy of it two metres the other way with a guid of its own, and the archetype's distance.
    let item = "    <Item type=\"CEntityDef\">";
    let end = "  </entities>";
    let source = folder.join("sources/harbour.metadata.src/metadata.xml");
    let xml = std::fs::read_to_string(&source).unwrap();
    let (from, to) = (xml.find(item).unwrap(), xml.find(end).unwrap());
    let copy = xml[from..to]
        .replace("x=\"10.0\"", "x=\"8.0\"")
        .replace("<guid value=\"1\"/>", "<guid value=\"2\"/>");
    assert_ne!(copy, xml[from..to]);
    std::fs::write(&source, xml.replace(end, &format!("{copy}{end}"))).unwrap();
    edit(&folder, "harbour", "x=\"10.0\"", "x=\"12.0\"");
    edit(
        &folder,
        "harbour_types",
        "<lodDist value=\"100.0\"/>",
        "<lodDist value=\"101.0\"/>",
    );
    file
}

/// The FiveM resource the recipe builds from [`the_sample`], with the three files it ships.
fn the_built_sample(work: &Path) -> PathBuf {
    the_sample(work);
    let said = run_step("build", work);
    let resource = work.join("acceptance/build/or_acceptance");
    for shipped in [
        "stream/harbour.ymap",
        "stream/harbour_types.ytyp",
        "fxmanifest.lua",
    ] {
        assert!(resource.join(shipped).is_file(), "{shipped} ships: {said}");
    }
    resource
}

#[test]
fn the_recipe_builds_a_resource_and_the_command_names_the_lines_about_its_files() {
    let work = tempfile::tempdir().unwrap();
    let resource = the_built_sample(work.path());

    // What the lines send a person to look for is in the files: the object two metres on, its copy two metres back, the archetype changed.
    let map = std::fs::read(resource.join("stream/harbour.ymap")).unwrap();
    let map = CMapData::from_bytes(&map).expect("the built map opens");
    let placed: Vec<(f32, u32)> = (map.entities.iter())
        .map(|entity| (entity.position.x, entity.guid))
        .collect();
    assert_eq!(placed, [(12.0, 1), (8.0, 2)]);
    let types = std::fs::read(resource.join("stream/harbour_types.ytyp")).unwrap();
    let types = CMapTypes::from_bytes(&types).expect("the built type file opens");
    let distances: Vec<f32> = types.archetypes.iter().map(|a| a.lod_dist).collect();
    assert_eq!(distances, [101.0]);

    let printed = run_step("acceptance", work.path());
    let (folder, _) = printed
        .split_once(": 3 file(s) of 3 kind(s)\n")
        .unwrap_or_else(|| panic!("the command counts what the build wrote: {printed}"));
    assert_eq!(
        Path::new(folder),
        resource.as_path(),
        "the command reads the resource the build wrote: {printed}"
    );
    let sheet = Sheet::shipped().unwrap();
    let kinds: BTreeSet<String> = ["ymap", "ytyp", "lua"].map(str::to_string).into();
    let about = sheet.about(&kinds, Target::Legacy);
    assert!(!about.is_empty());
    for line in &sheet.lines {
        let named = printed.contains(&format!(
            "  {} (.{}, 1 file(s); {})",
            line.id, line.kind, line.row
        ));
        assert_eq!(
            named,
            about.iter().any(|concern| concern.line.id == line.id),
            "{}:\n{printed}",
            line.id
        );
    }
    // ⚠ The command prints the sheet the tree ships, which a person writes results into: a line is not checked for an edition
    // while the sheet holds no run of it there.
    let unchecked: usize = [Target::Legacy, Target::Enhanced]
        .map(|edition| {
            let about = sheet.about(&kinds, edition);
            about.iter().filter(|c| c.newest.is_none()).count()
        })
        .iter()
        .sum();
    assert_eq!(
        printed.matches("not checked in game").count(),
        unchecked,
        "{printed}"
    );

    let (read, printed) = cli(&["project", "acceptance", text(&work.path().join("gone"))]);
    assert!(
        !read && printed.is_empty(),
        "a folder that is not there is refused: {printed}"
    );
}

#[test]
fn the_build_of_the_recipe_says_which_kinds_nobody_saw_loading_as_the_sheet_reads_them() {
    let work = tempfile::tempdir().unwrap();
    the_sample(work.path());
    let said = run_step("build", work.path());
    let resource = work
        .path()
        .join("acceptance")
        .join("build")
        .join("or_acceptance");

    // The recipe makes its project for Legacy, so the build is read for that edition alone.
    let kinds: BTreeSet<String> = kinds_in(&resource).unwrap().into_keys().collect();
    // ⚠ Read from the sheet the tree ships, which a person writes results into: what the build says follows the sheet, by the
    // results from a server, where a resource loads.
    let sheet = Sheet::shipped().unwrap();
    let seen = sheet.seen(&kinds, Some(Target::Legacy), Place::FivemServer);
    let [(edition, reading)] = seen.as_slice() else {
        panic!("the build wrote a map and a type file, which the sheet has lines about: {seen:?}");
    };
    let sentence = format!("  {}\n", reading.sentence(*edition));
    assert_eq!(said.matches(&sentence).count(), 1, "{sentence}{said}");
    assert!(sentence.contains("GTA V Legacy"), "{sentence}");
    assert!(!said.contains("GTA V Enhanced"), "{said}");
    let no_line = sheet
        .silent_sentence(&kinds)
        .expect("the manifest has no line");
    assert!(
        said.contains(&format!("  {no_line}\n")),
        "{no_line}\n{said}"
    );
    let to_check = format!(
        "  to check: openrage-cli project acceptance \"{}\" lists what to look for",
        text(&resource)
    );
    // The steps are named while a kind is left to check, and no longer once every line was seen.
    let left = matches!(reading, Seen::Not(_));
    assert_eq!(said.contains(&to_check), left, "{to_check}\n{said}");
    // What the build read back comes first, and the closing line still says what a build is not.
    let (read_back, in_game, closing) = (
        said.find("  names: ").expect("the names the build checked"),
        said.find(&sentence).unwrap(),
        said.find("This checks the files. It does not test that GTA V loads them.")
            .expect("the closing line"),
    );
    assert!(read_back < in_game && in_game < closing, "{said}");
}

/// A project whose one clothing content holds a jacket dropped on it, and which builds a singleplayer DLC named `name` for no stated edition.
fn a_clothing_pack_project(dir: &Path, name: &str) -> PathBuf {
    use or_project::{Container, Module, Output, OutputKind, Project};

    let pack = dir.join("pack");
    or_wardrobe::edit::create(&pack, name).unwrap();
    let drop = dir.join("drop");
    std::fs::create_dir_all(&drop).unwrap();
    let c = format!("mp_m_freemode_01_mp_m_{name}");
    std::fs::write(
        drop.join(format!("{c}^jbib_000_u.ydd")),
        or_source::sample::drawables_of("jacket"),
    )
    .unwrap();
    std::fs::write(
        drop.join(format!("{c}^jbib_diff_000_a_uni.ytd")),
        or_source::sample::textures_of("jacket"),
    )
    .unwrap();
    or_wardrobe::edit::add_clothes(&pack, &[drop]).unwrap();
    let container = Container {
        modules: vec![Module::clothing("wardrobe", "pack")],
        outputs: vec![Output::of(
            OutputKind::SingleplayerDlc,
            "dlc",
            name,
            &["wardrobe"],
        )],
    };
    let path = dir.join(format!("{name}.openrage-proj"));
    Project::create(&path, &container, &[]).unwrap();
    path
}

#[test]
fn the_build_of_a_clothing_pack_says_that_nobody_saw_a_story_pack_loading_from_either_build_verb() {
    let work = tempfile::tempdir().unwrap();
    let project = a_clothing_pack_project(work.path(), "demo");
    let folder = work.path().join("build").join("demo");
    let to_check = format!(
        "  to check: openrage-cli project acceptance \"{}\" lists what to look for",
        text(&folder)
    );
    for family in ["wardrobe", "project"] {
        let (built, said) = cli(&[family, "build", text(&project)]);
        assert!(built, "{family} build: {said}");

        // The pack states no edition, so it is read for each, and the archive is the one kind a line is about.
        let kinds: BTreeSet<String> = kinds_in(&folder).unwrap().into_keys().collect();
        assert!(kinds.contains("rpf") && kinds.contains("ydd"), "{kinds:?}");
        // ⚠ Read from the sheet the tree ships, by the results from story mode, where a pack loads.
        let seen = Sheet::shipped()
            .unwrap()
            .seen(&kinds, None, Place::StoryMode);
        assert_eq!(seen.len(), 2, "{seen:?}");
        for (edition, reading) in &seen {
            let sentence = format!("  {}\n", reading.sentence(*edition));
            if let Seen::Not(unseen) = reading {
                assert_eq!(unseen, &["rpf"], "{sentence}");
                assert!(sentence.contains("singleplayer DLC packs"), "{sentence}");
            }
            assert_eq!(
                said.matches(&sentence).count(),
                1,
                "{family} build: {sentence}{said}"
            );
        }
        let left = (seen.iter()).any(|(_, reading)| matches!(reading, Seen::Not(_)));
        assert_eq!(
            said.contains(&to_check),
            left,
            "{family} build: {to_check}\n{said}"
        );
    }
}

#[test]
fn the_sheet_and_the_recipe_are_about_the_same_kinds_of_file() {
    let work = tempfile::tempdir().unwrap();
    let resource = the_built_sample(work.path());
    let kinds: BTreeSet<String> = kinds_in(&resource).unwrap().into_keys().collect();
    let sheet = Sheet::shipped().unwrap();

    // A kind no build reads back, such as the manifest's, can have no line.
    let silent: Vec<&str> = (sheet.without_a_line(&kinds).into_iter())
        .filter(|kind| knows(kind))
        .collect();
    assert!(
        silent.is_empty(),
        "the recipe builds files the sheet has no line about: {silent:?}"
    );
    // ⚠ The archive of a pack is made by the guide's story mode steps, from content a pack carries, and not by the recipe.
    let pack = kind_of("dlc.rpf");
    let unmade: Vec<&str> = (sheet.lines.iter())
        .filter(|line| !kinds.contains(&line.kind) && Some(&line.kind) != pack.as_ref())
        .map(|line| line.id.as_str())
        .collect();
    assert!(
        unmade.is_empty(),
        "the recipe builds no file of the kind these lines are about: {unmade:?}"
    );
}

#[test]
fn a_story_pack_refuses_the_sample_as_the_guide_says_it_does_today() {
    let work = tempfile::tempdir().unwrap();
    let file = the_sample(work.path());
    let (added, said) = cli(&[
        "project",
        "add-build",
        text(&file),
        "--kind",
        "dlc",
        "--name",
        "or_acceptance_story",
    ]);
    assert!(added, "{said}");
    let (built, said) = cli(&[
        "project",
        "build",
        text(&file),
        "--build",
        "or_acceptance_story",
    ]);
    // ⚠ When this builds, a story pack carries a map: the guide's story mode section and the sheet gain what R70q writes.
    assert!(
        !built,
        "a singleplayer DLC is locked for a map and a type file: {said}"
    );
    assert!(
        said.contains("stream/harbour.ymap") && said.contains("stream/harbour_types.ytyp"),
        "the refusal names the two files: {said}"
    );
    assert!(!work
        .path()
        .join("acceptance/build/or_acceptance_story")
        .exists());
}
