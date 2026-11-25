//! The managed `.gitignore` block follows what the owner chose to commit: the sources, the build, the sources taken from the game.

use std::fs;
use std::path::Path;

use or_source::git::{self, Commit};

/// The lines of the managed block of `.gitignore`, markers included.
fn block(root: &Path) -> Vec<String> {
    let text = fs::read_to_string(root.join(git::IGNORE)).unwrap();
    let mut inside = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line == git::BEGIN {
            inside = true;
        }
        if inside {
            lines.push(line.to_string());
        }
        if line == git::END {
            break;
        }
    }
    lines
}

/// A project folder with a source taken from the game and one a person made.
fn a_project(root: &Path) {
    for (name, from_game) in [("theirs", true), ("mine", false)] {
        let dir = root.join(format!("sources/{name}.textures.src"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("asset.json"),
            format!(
                r#"{{"created_with":"t","format":"openrage-source","id":"7c1e2b0a-5d1f-4e7a-9b3c-2a6f0d8e4c1{}","kind":"textures","name":"{name}","notes":"","origin":{{"from_game":{from_game},"how":"{}"}},"platform":"win64","schema":1,"target":"gen8"}}"#,
                if from_game { 1 } else { 2 },
                if from_game { "extracted" } else { "authored" },
            ),
        )
        .unwrap();
    }
}

#[test]
fn what_the_owner_commits_is_written_in_the_block_and_kept_by_the_next_update() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    a_project(root);

    // As before: the sources, not the build, not what came from the game.
    assert_eq!(git::commit_of(root), Commit::default());
    git::update(root).unwrap();
    let default = block(root);
    assert_eq!(
        default,
        [
            git::BEGIN,
            "/build/",
            "/vfs/",
            git::KEPT_MANIFESTS,
            "/sources/theirs.textures.src/",
            git::END
        ]
    );

    // The build instead of the sources.
    let build = Commit {
        sources: false,
        build: true,
        game_sources: false,
    };
    assert!(git::write_with(root, false, build).unwrap());
    let lines = block(root);
    assert!(!lines.contains(&"/build/".to_string()), "{lines:?}");
    assert!(lines.contains(&"/sources/".to_string()), "{lines:?}");
    assert!(lines.contains(&"# commit: build".to_string()), "{lines:?}");
    assert_eq!(git::commit_of(root), build);
    // An import or a new project rewrites the block as it finds it, and keeps the choice.
    assert!(!git::update(root).unwrap());
    assert_eq!(git::commit_of(root), build);

    // The sources, with those taken from the game.
    let both = Commit {
        sources: true,
        build: false,
        game_sources: true,
    };
    assert!(git::write_with(root, false, both).unwrap());
    let lines = block(root);
    assert!(lines.contains(&"/build/".to_string()), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("theirs")), "{lines:?}");
    assert!(!lines.contains(&"/sources/".to_string()), "{lines:?}");
    assert_eq!(git::commit_of(root), both);

    // Back to the default, which writes the block it always wrote.
    assert!(git::write_with(root, false, Commit::default()).unwrap());
    assert_eq!(block(root), default);
    assert_eq!(git::commit_of(root), Commit::default());
}

#[test]
fn a_line_of_the_person_outside_the_block_survives_a_new_choice() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(root.join(git::IGNORE), "dist/\n").unwrap();
    git::write_with(
        root,
        false,
        Commit {
            sources: true,
            build: true,
            game_sources: false,
        },
    )
    .unwrap();
    let text = fs::read_to_string(root.join(git::IGNORE)).unwrap();
    assert!(text.starts_with("dist/\n"), "{text}");
    assert!(!block(root).contains(&"/build/".to_string()));
}
