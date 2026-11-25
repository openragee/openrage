//! `openrage-cli project check-server` reads a server's resources folder and answers with its exit code: 0 where no rule finds a warning,
//! 1 where one does, 2 for a command line it cannot read. `project check-build` asks the same folder what a build of a project is about to
//! write again. What each prints is its report's lines, and with `--json` the same findings as data.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_io::meta::instance::HashedString;
use or_maps::import::{add_files, Incoming};
use or_maps::sample::{map_file, types_file, Archetype, Entity};

fn project(args: &[&str]) -> ExitCode {
    let mut all = vec!["project".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
}

fn check(args: &[&str]) -> ExitCode {
    project(&[&["check-server"][..], args].concat())
}

const CHILD_TEST: &str = "the_child_that_runs_one_command";
const CHILD_ENV: &str = "OPENRAGE_TEST_ONE_COMMAND";
const BEGIN: &str = "<<begin>>";
const END: &str = "<<end>>";
const SEPARATOR: char = '\u{1f}';

/// Run by `printed` in a child process, so that what the command prints can be read.
#[test]
fn the_child_that_runs_one_command() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args = args.to_string_lossy().into_owned();
    let args: Vec<&str> = args.split(SEPARATOR).collect();
    let mut out = std::io::stdout();
    let _ = write!(out, "{BEGIN}");
    let _ = out.flush();
    let code = project(&args);
    let _ = out.flush();
    let _ = write!(out, "{END}{}", u8::from(code == ExitCode::SUCCESS));
    let _ = out.flush();
}

/// Whether `project <args>` exited zero, and what it printed.
fn printed(args: &[&str]) -> (bool, String) {
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

fn put(at: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, bytes).unwrap();
}

/// A resource at `path` under `root` with a manifest and the files given under its stream folder.
fn resource(root: &Path, path: &str, streamed: &[(&str, &[u8])]) {
    put(
        &root.join(path).join("fxmanifest.lua"),
        b"fx_version 'cerulean'\ngame 'gta5'\nthis_is_a_map 'yes'\n",
    );
    for (name, bytes) in streamed {
        put(&root.join(path).join("stream").join(name), bytes);
    }
}

fn harbour_map(name: &str) -> Vec<u8> {
    map_file(
        name,
        &[Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1)],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    )
}

fn crate_types(name: &str) -> Vec<u8> {
    types_file(name, &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])])
}

/// A server folder whose `resources` hold two resources that stream one dictionary with the same bytes and nothing else of one name.
fn a_server_with_no_warning(server: &Path) -> PathBuf {
    let root = server.join("resources");
    resource(
        &root,
        "[maps]/docks",
        &[
            ("harbour.ymap", &harbour_map("harbour")),
            ("docks.ytyp", &crate_types("docks")),
            ("shared.ytd", b"one dictionary"),
        ],
    );
    resource(&root, "yard", &[("shared.ytd", b"one dictionary")]);
    put(
        &server.join("server.cfg"),
        b"sv_maxclients 48\nensure yard\nensure [maps]\n",
    );
    root
}

#[test]
fn a_server_with_only_notes_exits_zero_and_a_warning_exits_one_as_text_and_as_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = a_server_with_no_warning(dir.path());
    let cfg = dir.path().join("server.cfg");
    let report =
        or_cli::check_server_folder(&root, Some(&cfg), None, None).expect("a folder reads");
    let codes: Vec<&str> = report.findings.iter().map(|f| f.code).collect();
    assert_eq!(
        codes,
        ["file-in-two-resources-same", "configuration-line-unread"],
        "the same bytes twice and a line of the configuration are notes: {:#?}",
        report.findings
    );
    assert_eq!(check(&[text(&root)]), ExitCode::SUCCESS);
    assert_eq!(
        check(&[text(&root), "--cfg", text(&cfg)]),
        ExitCode::SUCCESS
    );
    assert_eq!(check(&[text(&root), "--json"]), ExitCode::SUCCESS);

    // Another copy of the map with other bytes is a warning, which fails the check.
    resource(
        &root,
        "yard",
        &[("harbour.ymap", &harbour_map("harbour_moved"))],
    );
    assert_eq!(check(&[text(&root)]), ExitCode::from(1));
    assert_eq!(check(&[text(&root), "--json"]), ExitCode::from(1));
    assert_eq!(
        check(&[text(&root), "--cfg", text(&cfg)]),
        ExitCode::from(1)
    );
}

#[test]
fn a_command_line_that_cannot_be_read_is_a_usage_error_and_a_path_that_is_not_there_is_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let root = a_server_with_no_warning(dir.path());
    let cfg = dir.path().join("server.cfg");
    assert_eq!(check(&[]), ExitCode::from(2));
    assert_eq!(check(&[text(&root), text(&root)]), ExitCode::from(2));
    assert_eq!(check(&[text(&root), "--strict"]), ExitCode::from(2));
    assert_eq!(check(&[text(&root), "--cfg"]), ExitCode::from(2));
    assert_eq!(check(&[text(&cfg)]), ExitCode::FAILURE);
    assert_eq!(check(&[text(&dir.path().join("gone"))]), ExitCode::FAILURE);
    assert_eq!(
        check(&[text(&root), "--cfg", text(&dir.path().join("gone.cfg"))]),
        ExitCode::FAILURE
    );
    assert_eq!(
        check(&[text(&root), "--cfg", text(&root)]),
        ExitCode::FAILURE
    );
    // One resource is not a server's folder of them: read as one, it would hold no resource and report nothing.
    let one = root.join("yard");
    assert_eq!(check(&[text(&one)]), ExitCode::FAILURE);
    let refused = or_cli::check_server_folder(&one, None, None, None);
    assert!(refused.is_err_and(|why| why.contains("is one resource")));
    let refused = or_cli::check_server_folder(&cfg, None, None, None);
    assert!(refused.is_err_and(|why| why.contains("is not a folder")));
    let refused = or_cli::check_server_folder(&root, Some(&root), None, None);
    assert!(refused.is_err_and(|why| why.contains("is not a file")));
    // Neither is the folder a server runs in, which holds the resources folder: read, it would hold no resource and pass.
    assert_eq!(check(&[text(dir.path())]), ExitCode::FAILURE);
    let refused = or_cli::check_server_folder(dir.path(), Some(&cfg), None, None);
    assert!(
        refused
            .as_ref()
            .is_err_and(|why| why.contains("holds no resource")
                && why.contains("its folder `resources` may be the one")),
        "{:?}",
        refused.map(|report| report.resources.len())
    );
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(empty.join("plain/inner")).unwrap();
    let refused = or_cli::check_server_folder(&empty, None, None, None);
    assert!(refused.is_err_and(|why| why.ends_with("has a manifest")));
}

#[test]
fn what_is_printed_names_both_resources_and_the_one_that_starts_last_and_the_json_lists_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let root = a_server_with_no_warning(dir.path());
    resource(
        &root,
        "yard",
        &[("harbour.ymap", &harbour_map("harbour_moved"))],
    );
    let cfg = dir.path().join("server.cfg");

    let (ok, plain) = printed(&["check-server", text(&root)]);
    assert!(!ok, "a warning fails the check");
    let warning = (plain.lines())
        .find(|line| line.contains("file-in-two-resources "))
        .unwrap_or_else(|| panic!("no line for the map: {plain}"));
    assert!(
        warning.trim_start().starts_with("warning")
            && warning
                .contains("`[maps]/docks` and `yard` stream `harbour.ymap` with different bytes")
            && warning.contains("not known: no configuration was given"),
        "{warning}"
    );
    assert!(plain.contains("configuration: none given"), "{plain}");

    let (ok, ordered) = printed(&["check-server", text(&root), "--cfg", text(&cfg)]);
    assert!(!ok);
    assert!(
        ordered.contains("`[maps]/docks` starts last, on line 3 of `server.cfg`"),
        "{ordered}"
    );

    let (ok, json) = printed(&["check-server", text(&root), "--cfg", text(&cfg), "--json"]);
    assert!(!ok);
    let documents: serde_json::Value = serde_json::from_str(&json).expect("the JSON parses");
    let stated = documents[0]["findings"].as_array().expect("the findings");
    let lines: Vec<&str> = (ordered.lines())
        .filter(|line| {
            ["  warning ", "  note ", "  error "]
                .iter()
                .any(|word| line.starts_with(word))
        })
        .collect();
    assert_eq!(stated.len(), 3, "{json}");
    assert_eq!(lines.len(), stated.len(), "{ordered}");
    for (line, finding) in lines.iter().zip(stated) {
        let code = finding["code"].as_str().expect("a code");
        let message = finding["message"].as_str().expect("a sentence");
        assert!(
            line.contains(code) && line.ends_with(message),
            "{line}\n{finding:#}"
        );
    }
    assert_eq!(stated[0]["starts_last"], "[maps]/docks");
    assert_eq!(
        stated[0]["resources"],
        serde_json::json!(["[maps]/docks", "yard"])
    );
    assert_eq!(stated[0]["names"], serde_json::json!(["harbour.ymap"]));

    // A file of a resource that the configuration includes is read inside that resource, and its lines start what they name.
    put(&root.join("yard/start.cfg"), b"ensure yard\n");
    put(&cfg, b"ensure [maps]\nexec @yard/start.cfg\n");
    let report = or_cli::check_server_folder(&root, Some(&cfg), None, None).expect("reads");
    let clash = (report.findings.iter())
        .find(|f| f.code == "file-in-two-resources")
        .expect("the map two resources stream");
    assert_eq!(clash.starts_last.as_deref(), Some("yard"));
    assert!(
        clash.message.contains("line 1 of `@yard/start.cfg`"),
        "{}",
        clash.message
    );
}

/// A project with a maps content whose pack holds a map file and the type file that declares what it places.
fn a_project_with_maps(dir: &Path) -> PathBuf {
    put(
        &dir.join("harbour.openrage-proj"),
        br#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "harbour", "take": ["maps"] }],
  "schema": 4
}"#,
    );
    let (map, types) = (harbour_map("harbour"), crate_types("harbour_types"));
    let incoming = |name, bytes| Incoming {
        name,
        bytes,
        from_game: false,
        virtual_path: None,
    };
    let added = add_files(
        &dir.join("maps"),
        &[
            incoming("harbour.ymap", &map),
            incoming("harbour_types.ytyp", &types),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    dir.join("harbour.openrage-proj")
}

#[test]
fn a_build_is_asked_what_a_server_already_holds_of_it_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project_with_maps(&dir.path().join("project"));
    let root = a_server_with_no_warning(&dir.path().join("server"));

    let found = or_cli::check_build_against(&file, None, None, &root).expect("the project plans");
    assert_eq!((found.resources, found.whole), (2, true));
    let [build] = &found.builds[..] else {
        panic!("one build");
    };
    assert_eq!(
        (build.build.as_str(), build.resource.as_str()),
        ("fivem", "harbour")
    );
    let stated: Vec<(&str, &[String], &[String])> = (build.findings.iter())
        .map(|f| (f.code, f.resources.as_slice(), f.names.as_slice()))
        .collect();
    let docks = ["[maps]/docks".to_string()];
    // The sources were made from files that keep an archetype's name as its hash, and so does the claim.
    let crate_a = [HashedString::new("prop_crate_a").to_literal()];
    assert_eq!(
        stated,
        [
            ("build-file-streamed-elsewhere", &docks[..], &["harbour.ymap".to_string()][..]),
            ("build-archetype-declared-elsewhere", &docks[..], &crate_a[..]),
        ],
        "the map the build cooks and the archetype its type file declares, and nothing of the dictionary two others share"
    );
    assert!(
        !dir.path().join("project/build").exists(),
        "the question builds nothing"
    );

    let args = ["check-build", text(&file), "--server", text(&root)];
    assert_eq!(project(&args), ExitCode::from(1));
    assert_eq!(
        project(&[&args[..], &["--json"][..]].concat()),
        ExitCode::from(1)
    );
    assert_eq!(
        project(&[&args[..], &["--build", "fivem"][..]].concat()),
        ExitCode::from(1)
    );
    assert_eq!(
        project(&[&args[..], &["--build", "absent"][..]].concat()),
        ExitCode::FAILURE
    );
    assert_eq!(project(&["check-build", text(&file)]), ExitCode::from(2));
    assert_eq!(
        project(&["check-build", "--server", text(&root)]),
        ExitCode::from(2)
    );
    assert_eq!(
        project(&["check-build", text(&file), "--server", text(&file)]),
        ExitCode::FAILURE
    );

    let (ok, plain) = printed(&args);
    assert!(!ok);
    let (_, json) = printed(&[&args[..], &["--json"][..]].concat());
    let documents: serde_json::Value = serde_json::from_str(&json).expect("the JSON parses");
    let listed = documents[0]["findings"].as_array().expect("the findings");
    assert_eq!(listed.len(), 2);
    for finding in listed {
        let message = finding["message"].as_str().expect("a sentence");
        assert!(
            plain.lines().any(|line| line.ends_with(message)),
            "{message}\n{plain}"
        );
    }
    assert_eq!(documents[0]["build"], "fivem");

    // What was not read of the server is said beside the answer, as text and as data.
    assert!(!plain.contains("this is not all it holds"), "{plain}");
    assert_eq!(documents[0]["whole"], true);
    resource(
        &root,
        "sealed",
        &[("sealed.ytyp", b"no type file a reader opens")],
    );
    let (_, plain) = printed(&args);
    assert!(
        plain.contains("was not read, so this is not all it holds"),
        "{plain}"
    );
    let (_, json) = printed(&[&args[..], &["--json"][..]].concat());
    let documents: serde_json::Value = serde_json::from_str(&json).expect("the JSON parses");
    assert_eq!(documents[0]["whole"], false);
    assert_eq!(documents[0]["resources"], 3);

    // A folder that holds no resource answers nothing, and is refused rather than read as a server with no clash.
    assert_eq!(
        project(&["check-build", text(&file), "--server", text(dir.path())]),
        ExitCode::FAILURE
    );
    assert_eq!(
        project(&[
            "check-build",
            text(&file),
            "--server",
            text(&root.join("yard"))
        ]),
        ExitCode::FAILURE
    );

    // With the server's copies gone, the build is told nothing and the check passes.
    std::fs::remove_dir_all(root.join("[maps]")).unwrap();
    let found = or_cli::check_build_against(&file, None, None, &root).expect("the project plans");
    assert_eq!(found.builds[0].findings, []);
    assert_eq!(project(&args), ExitCode::SUCCESS);
    // The build's own earlier copy in the server's folder is the resource it replaces.
    resource(
        &root,
        "harbour",
        &[("harbour.ymap", &harbour_map("harbour"))],
    );
    assert_eq!(project(&args), ExitCode::SUCCESS);

    // Written over that copy the build replaces it; written under a plain folder of the server it is where no server looks,
    // beside the copy it no longer replaces.
    let over = [&args[..], &["--out", text(&root)][..]].concat();
    assert_eq!(project(&over), ExitCode::SUCCESS);
    let plain = root.join("plain");
    let placed =
        or_cli::check_build_against(&file, None, Some(&plain), &root).expect("the project plans");
    let stated: Vec<(&str, &[String])> = (placed.builds[0].findings.iter())
        .map(|f| (f.code, f.names.as_slice()))
        .collect();
    assert_eq!(
        stated,
        [
            ("build-folder-outside-category", &["plain".to_string()][..]),
            ("build-resource-elsewhere", &["harbour".to_string()][..]),
        ]
    );
    let under = [&args[..], &["--out", text(&plain)][..]].concat();
    assert_eq!(project(&under), ExitCode::from(1));
    let (_, plain) = printed(&under);
    assert!(
        plain.contains("`plain` is no category in brackets, so a server does not look in it"),
        "{plain}"
    );
}
