//! A test run leaves nothing in the owner's temporary folder.
//!
//! ⚠ Read from the source rather than measured: a folder a test names by process id and removes
//! last is left by every failed run, and by every run nextest starts a process for and a test never
//! reaches the end of. A folder `tempfile` makes is removed when it drops, a failed test's too.
//! What each crate leaves after its run is measured by running the workspace with `TEMP` pointed at
//! an empty folder.

use std::path::{Path, PathBuf};

/// The calls that make a folder or file nothing removes.
const CALLS: [&str; 3] = [
    concat!("temp", "_dir()"),
    concat!(".ke", "ep()"),
    concat!(".into", "_path()"),
];

/// Where one of them is allowed, and why that leaves nothing.
const ALLOWED: &[(&str, &str)] = &[
    (
        "src/ui/os_drop.rs",
        "the editor's own staging folder, which a self-check and a test build keep out of",
    ),
    (
        "src/state/prefs.rs",
        "the folder a self-check names for its scratch, which the run removes however it ends",
    ),
    (
        "src/photo_serve.rs",
        "the folder a served run names for its scratch, which the run removes when it ends",
    ),
    (
        "src/state/game_pack.rs",
        "where the game pack goes when the platform has no configuration folder",
    ),
    (
        "src/self_check/project.rs",
        "the scratch project of the self-check, which its units remove when they end",
    ),
    (
        "src/self_check/wardrobe.rs",
        "the scratch clothing project of the self-check, which its units remove when they end",
    ),
    (
        "src/photo.rs",
        "the scratch folder of a photograph run, which its drop guard removes however the run ends",
    ),
    (
        "src/state/test_scratch.rs",
        "the one scratch folder of a test process, removed as the process ends",
    ),
    (
        "src/state/a_self_check_keeps_to_itself.rs",
        "lists the editor's staging folder to see that a self-check added nothing to it",
    ),
    (
        "src/state/a_plugin_asks_before_it_runs.rs",
        "names a document's path outside every install and writes nothing there",
    ),
    (
        "src/state/plugins/a_plugin_adds_to_the_editor.rs",
        "names a document's path and writes nothing there",
    ),
    (
        "crates/title/or-title/tests/one_path_across_archive_boundaries.rs",
        "points the process at a private folder of its own and counts what lands in it",
    ),
    (
        "crates/io/or-io-verify/src/bin/scaffold.rs",
        "an ignored test, run by name, that keeps a build cache between runs by design",
    ),
    (
        "crates/tools/or-cli/src/mcp.rs",
        "the pictures of a `shots` call that names no folder, which its caller reads afterwards and no test makes; and a test's path to an install that is not there, where nothing is written",
    ),
    (
        "crates/tools/or-cli/src/gta6_rdna.rs",
        "the scratch of one scan a person runs, which the command removes when it ends and no test makes",
    ),
    (
        "crates/tools/or-shader-tool/src/rdna_devices.rs",
        "the scratch of one device check a person runs, which the command removes when it ends and no test makes",
    ),
];

fn root() -> PathBuf {
    or_test_support::workspace_root(env!("CARGO_MANIFEST_DIR")).expect("a workspace above us")
}

/// A folder that holds no workspace code: build output, and the harness folders (`.claude`,
/// `.cursor`, `.opencode`) that .gitignore names because a harness's worktree is a second copy of
/// this repository inside it, holding another agent's old sources.
fn holds_no_workspace_code(name: &str) -> bool {
    name.starts_with('.')
        || name == "target"
        || name.starts_with("target-")
        || ["external", "docs", "node_modules"].contains(&name)
}

/// Every `.rs` file of the workspace's own code, as a path from `root` with forward slashes.
fn sources(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder)
            .expect("a folder lists")
            .flatten()
        {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if !holds_no_workspace_code(&name.to_string_lossy()) {
                    folders.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let relative = path.strip_prefix(root).expect("below the root");
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

fn calls_in(root: &Path, file: &str) -> Vec<(usize, &'static str)> {
    let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
    let mut calls = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or(line);
        calls.extend(
            CALLS
                .iter()
                .filter(|call| code.contains(**call))
                .map(|call| (index + 1, *call)),
        );
    }
    calls
}

#[test]
fn no_test_makes_a_folder_in_the_temporary_folder_that_nothing_removes() {
    let root = root();
    let files = sources(&root);
    assert!(
        files.len() > 500,
        "only {} files read: the walk is wrong",
        files.len()
    );
    let mut leaving = Vec::new();
    for file in &files {
        if ALLOWED.iter().any(|(allowed, _)| allowed == file) {
            continue;
        }
        for (line, call) in calls_in(&root, file) {
            leaving.push(format!("  {file}:{line}: {call}"));
        }
    }
    assert!(
        leaving.is_empty(),
        "these name a folder under the temporary folder that stays after the run; make it with \
         `tempfile::tempdir()` and hold it until the test ends:\n{}",
        leaving.join("\n")
    );
}

#[test]
fn a_harness_worktree_and_a_cargo_target_folder_are_not_workspace_code() {
    let scratch = tempfile::tempdir().expect("a folder");
    for file in [
        "crates/a/src/lib.rs",
        ".claude/worktrees/agent-1/crates/a/src/lib.rs",
        ".cursor/worktrees/agent-2/src/main.rs",
        "target/debug/build/b.rs",
        "target-lane/debug/build/b.rs",
    ] {
        let path = scratch.path().join(file);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        std::fs::write(path, "").expect("a file");
    }

    assert_eq!(sources(scratch.path()), ["crates/a/src/lib.rs"]);
}

#[test]
fn every_place_this_allows_still_makes_the_call_it_allows() {
    let root = root();
    let stale: Vec<&str> = ALLOWED
        .iter()
        .filter(|(file, _)| calls_in(&root, file).is_empty())
        .map(|(file, _)| *file)
        .collect();
    assert!(
        stale.is_empty(),
        "these no longer make the call, so take them off the list: {stale:?}"
    );
}
