//! Each key snapshot is in this repository exactly once, where or-io bundles it
//! (D18, and the owner's ruling of 2026-10-08 for RDR2's), and nothing but its one embed reads it.
//!
//! ⚠ A second copy that came back renamed would be the same file, so the
//! content is checked as well as the name.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha1::{Digest, Sha1};

/// A snapshot this repository carries, by content. A digest of key material is not key material.
struct Snapshot {
    /// The file's name, and the name of nothing else in the tree.
    file: &'static str,
    /// The one copy, and the one file that embeds it.
    bundled: &'static str,
    embed: &'static str,
    sha1: &'static str,
    len: u64,
}

const SNAPSHOTS: [Snapshot; 4] = [
    Snapshot {
        file: "gtavi_xboxsx_magic.dat",
        bundled: "crates/io/or-io/data/gtavi_xboxsx_magic.dat",
        embed: "crates/io/or-io/src/container/rpf/gta6_keygate.rs",
        sha1: "8c488ed36ce5e49874a2dfac873b7248d71568a1",
        len: 144_248,
    },
    Snapshot {
        file: "gtavi_ps5_magic.dat",
        bundled: "crates/io/or-io/data/gtavi_ps5_magic.dat",
        embed: "crates/io/or-io/src/container/rpf/gta6_keygate.rs",
        sha1: "5681c8fa6d48b277a02da03cc2349cfb89281f0d",
        len: 144_260,
    },
    Snapshot {
        file: "magic.dat",
        bundled: "crates/io/or-io/data/magic.dat",
        embed: "crates/io/or-io/src/container/rpf/crypto/magic.rs",
        sha1: "0768e698862cce8ad3ca1683e6388f76dff26758",
        len: 154_069,
    },
    Snapshot {
        file: "rdr2_magic.dat",
        bundled: "crates/io/or-io/data/rdr2_magic.dat",
        embed: "crates/io/or-io/src/container/rpf/crypto/rdr2.rs",
        sha1: "7ed9a5b25abdcbdb99850ebe44e6e18b5784d101",
        len: 666_746,
    },
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crates/container/or-container sits three below the root")
        .to_path_buf()
}

/// Everything a clone gets plus everything one `git add -A` would stage.
fn committable_files(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .current_dir(root)
        .output();
    match output {
        Ok(out) if out.status.success() => out
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| root.join(String::from_utf8_lossy(s).as_ref()))
            .collect(),
        _ => walk(root),
    }
}

/// Without git, every file under the root except build output and git's own.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file = entry.file_name();
            if path.is_dir() {
                if file != "target" && file != ".git" {
                    stack.push(path);
                }
            } else {
                out.push(path);
            }
        }
    }
    out
}

fn is_known_snapshot(path: &Path, snapshot: &Snapshot) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.len() != snapshot.len {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let digest = Sha1::digest(&bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex == snapshot.sha1
}

/// Embeds of a file named like the snapshot, however rustfmt wraps the call.
fn snapshot_embeds(text: &str, file: &str) -> usize {
    let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    flat.match_indices(concat!("include", "_bytes!"))
        .filter(|(at, _)| {
            let rest = &flat[*at..];
            let call = &rest[..rest.find(';').unwrap_or(rest.len())];
            call.contains(&format!("/{file}\"")) || call.contains(&format!("\"{file}\""))
        })
        .count()
}

fn offenders(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for path in committable_files(root) {
        if SNAPSHOTS.iter().any(|s| path == root.join(s.bundled)) {
            continue;
        }
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        for snapshot in &SNAPSHOTS {
            if file == snapshot.file {
                found.push(format!("{}: named like a key snapshot", path.display()));
            } else if is_known_snapshot(&path, snapshot) {
                found.push(format!("{}: the key snapshot, renamed", path.display()));
            }
        }
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                for snapshot in &SNAPSHOTS {
                    let embeds = snapshot_embeds(&text, snapshot.file);
                    if path == root.join(snapshot.embed) {
                        if embeds != 1 {
                            found.push(format!("{}: {embeds} embeds, not one", path.display()));
                        }
                    } else if embeds > 0 {
                        found.push(format!("{}: embeds a key snapshot", path.display()));
                    }
                }
            }
        }
    }
    found
}

#[test]
fn the_key_snapshot_is_in_the_tree_once() {
    let root = workspace_root();
    for snapshot in &SNAPSHOTS {
        assert!(
            is_known_snapshot(&root.join(snapshot.bundled), snapshot),
            "the bundled key snapshot is missing or is not the known one: {}",
            snapshot.bundled
        );
    }
    let found = offenders(&root);
    assert!(
        found.is_empty(),
        "a second key snapshot is in the repository; the copies are the bundled ones:\n  {}",
        found.join("\n  ")
    );
}

/// The bundled path is the only place a copy may sit, whatever it is called elsewhere.
#[test]
fn a_second_copy_of_a_bundled_snapshot_is_found() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path();
    for (i, snapshot) in SNAPSHOTS.iter().enumerate() {
        let bytes = std::fs::read(workspace_root().join(snapshot.bundled)).unwrap();
        let allowed = dir.join(snapshot.bundled);
        std::fs::create_dir_all(allowed.parent().unwrap()).unwrap();
        std::fs::write(&allowed, &bytes).unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets").join(format!("keys{i}.bin")), &bytes).unwrap();
    }

    let found = offenders(dir);
    assert_eq!(found.len(), SNAPSHOTS.len(), "{found:#?}");
    assert!(found
        .iter()
        .all(|f| f.contains("keys") && f.contains("renamed")));
}

/// The check above is only worth something if it fires.
#[test]
fn the_guard_finds_a_planted_snapshot_by_name_and_by_embed() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("assets").join("Magic.DAT"), b"planted").unwrap();
    std::fs::write(dir.join("assets").join("RDR2_Magic.dat"), b"planted").unwrap();
    let embed = format!(
        "pub const K: &[u8] = {}(\"../assets/magic.dat\");\n",
        concat!("include", "_bytes!")
    );
    std::fs::write(dir.join("src").join("lib.rs"), embed).unwrap();
    let wrapped = format!(
        "pub const K: &[u8] = {}(concat!(\n    env!(\"CARGO_MANIFEST_DIR\"),\n    \"/data/rdr2_magic.dat\"\n));\n",
        concat!("include", "_bytes!")
    );
    std::fs::write(dir.join("src").join("wrapped.rs"), wrapped).unwrap();

    let found = offenders(dir);
    assert_eq!(found.len(), 4, "{found:#?}");
    assert_eq!(found.iter().filter(|f| f.contains("named like")).count(), 2);
    assert!(found
        .iter()
        .any(|f| f.contains("lib.rs") && f.contains("embeds")));
    assert!(found
        .iter()
        .any(|f| f.contains("wrapped.rs") && f.contains("embeds")));
}
