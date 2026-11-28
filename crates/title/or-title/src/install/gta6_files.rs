//! The files of a GTA VI install: every archive under a folder, with its place in the pair a
//! PlayStation 5 build ships most archives as, and every texture pack.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The most folders the walk reads, and the most archives and texture packs it lists.
pub const MOST_FOLDERS: usize = 100_000;
pub const MOST_ARCHIVES: usize = 1_000;
pub const MOST_TEXTURE_PACKS: usize = 10_000;

/// An archive's place in a pair, told by its name and the name beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// `X.rpf` with no `X_concat.rpf` beside it.
    Alone,
    /// `X.rpf` with `X_concat.rpf` beside it.
    First,
    /// `X_concat.rpf`, whether or not `X.rpf` lies beside it.
    Concat,
}

/// One archive of an install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub path: PathBuf,
    pub role: Role,
    /// The file name without its extension and without `_concat`, which both files of a pair share.
    // ⚠ Only the last `_concat` is cut: `story000_script_concat.rpf` is the `_concat` of
    // `story000_script`, with no first where no `story000_script.rpf` lies beside it, and is no
    // third file of `story000`.
    pub stem: String,
    /// The folder of `pack/` the archive lies directly in.
    pub pack: Option<String>,
}

/// What a walk of an install listed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Files {
    /// In path order, but a `_concat` right after the first of its pair, or where that would lie.
    pub archives: Vec<Archive>,
    /// In path order.
    pub texture_packs: Vec<PathBuf>,
    /// Whether the walk stopped at a limit before it listed every file.
    pub truncated: bool,
    /// The folders that did not read, whose files are not listed.
    pub unread: usize,
}

impl Files {
    /// The archives' paths, in the order listed.
    pub fn archive_paths(&self) -> Vec<PathBuf> {
        (self.archives.iter())
            .map(|archive| archive.path.clone())
            .collect()
    }
}

/// Every file named `.rpf` or `.ttpf` under `root`, at any depth, up to the limits; a folder that does not read is passed over.
pub fn list(root: &Path) -> Files {
    let mut found = Files::default();
    let (mut folders, mut read) = (vec![root.to_path_buf()], 0);
    while let Some(folder) = folders.pop() {
        read += 1;
        let Ok(entries) = std::fs::read_dir(&folder) else {
            found.unread += 1;
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            let ext = path.extension();
            let named = |name: &str| ext.is_some_and(|e| e.eq_ignore_ascii_case(name));
            let (rpf, ttpf) = (named("rpf"), named("ttpf"));
            if path.is_dir() {
                match read + folders.len() < MOST_FOLDERS {
                    true => folders.push(path),
                    false => found.truncated = true,
                }
            } else if rpf {
                match found.archives.len() < MOST_ARCHIVES {
                    true => found.archives.push(archive(root, path)),
                    false => found.truncated = true,
                }
            } else if ttpf {
                match found.texture_packs.len() < MOST_TEXTURE_PACKS {
                    true => found.texture_packs.push(path),
                    false => found.truncated = true,
                }
            }
        }
    }
    found.texture_packs.sort();
    let key = |archive: &Archive| (first_path(archive), archive.role == Role::Concat);
    found.archives.sort_by_cached_key(key);
    for at in 1..found.archives.len() {
        let (first, concat) = (&found.archives[at - 1], &found.archives[at]);
        let pair = first.role == Role::Alone
            && concat.role == Role::Concat
            && first.path == first_path(concat);
        if pair {
            found.archives[at - 1].role = Role::First;
        }
    }
    found
}

/// The paths of the archives the game mounts of the install at `root`, those at the root and those directly in a folder of `pack/`, in the order [`list`] gives.
pub fn mounted(root: &Path) -> Vec<PathBuf> {
    let at_root = |archive: &Archive| archive.path.parent() == Some(root);
    (list(root).archives.into_iter())
        .filter(|archive| archive.pack.is_some() || at_root(archive))
        .map(|archive| archive.path)
        .collect()
}

/// The archive at `path` under `root`, alone until a `_concat` is seen beside it.
fn archive(root: &Path, path: PathBuf) -> Archive {
    let whole = path.file_stem().unwrap_or_default().to_string_lossy();
    let (stem, role) = match whole.strip_suffix("_concat") {
        Some(stem) if !stem.is_empty() => (stem.to_string(), Role::Concat),
        _ => (whole.to_string(), Role::Alone),
    };
    let folder = path.parent();
    let in_pack = folder.and_then(Path::parent).is_some_and(|packs| {
        let named = (packs.file_name()).is_some_and(|name| name.eq_ignore_ascii_case("pack"));
        named && packs.parent() == Some(root)
    });
    let pack = (folder.filter(|_| in_pack).and_then(Path::file_name))
        .map(|name| name.to_string_lossy().into_owned());
    Archive {
        path,
        role,
        stem,
        pack,
    }
}

/// Where the first of `archive`'s pair lies or would lie: beside a `_concat`, else its own path.
fn first_path(archive: &Archive) -> PathBuf {
    match (archive.role, archive.path.extension()) {
        (Role::Concat, Some(extension)) => {
            let mut name = OsString::from(&archive.stem);
            name.push(".");
            name.push(extension);
            archive.path.with_file_name(name)
        }
        _ => archive.path.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder with an empty file at each of `files`, each a path under it.
    fn tree(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for file in files {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
            std::fs::write(path, []).expect("written");
        }
        dir
    }

    /// `path` under `root`, its folders joined by `/`.
    fn under(root: &Path, path: &Path) -> String {
        let under = path.strip_prefix(root).expect("under the root");
        under.to_string_lossy().replace('\\', "/")
    }

    type Row = (String, Role, String, Option<String>);

    fn rows(root: &Path, files: &Files) -> Vec<Row> {
        (files.archives.iter())
            .map(|a| (under(root, &a.path), a.role, a.stem.clone(), a.pack.clone()))
            .collect()
    }

    fn row(path: &str, role: Role, stem: &str, pack: Option<&str>) -> Row {
        (path.into(), role, stem.into(), pack.map(str::to_string))
    }

    #[test]
    fn an_xbox_shaped_install_lists_each_archive_alone_in_path_order_and_its_packs() {
        let dir = tree(&[
            "levels.rpf",
            "packs.rpf",
            "packs_mandy3.rpf",
            "notes.txt",
            "pack/story000/story000.rpf",
            "pack/story000/story000_script.rpf",
            "pack/story000/data/ttdb/maps/b.ttpf",
            "data/ttdb/a.TTPF",
            "kept/deep/other.RPF",
        ]);
        let root = dir.path();
        let files = list(root);
        let story = Some("story000");
        assert_eq!(
            rows(root, &files),
            [
                row("kept/deep/other.RPF", Role::Alone, "other", None),
                row("levels.rpf", Role::Alone, "levels", None),
                row("pack/story000/story000.rpf", Role::Alone, "story000", story),
                row(
                    "pack/story000/story000_script.rpf",
                    Role::Alone,
                    "story000_script",
                    story
                ),
                row("packs.rpf", Role::Alone, "packs", None),
                row("packs_mandy3.rpf", Role::Alone, "packs_mandy3", None),
            ]
        );
        let mut by_path = files.archive_paths();
        by_path.sort();
        assert_eq!(files.archive_paths(), by_path, "with no pair, path order");
        let packs: Vec<String> = (files.texture_packs.iter())
            .map(|path| under(root, path))
            .collect();
        assert_eq!(
            packs,
            ["data/ttdb/a.TTPF", "pack/story000/data/ttdb/maps/b.ttpf"]
        );
        assert!(!files.truncated);

        let mounted: Vec<String> = mounted(root).iter().map(|p| under(root, p)).collect();
        assert_eq!(
            mounted,
            [
                "levels.rpf",
                "pack/story000/story000.rpf",
                "pack/story000/story000_script.rpf",
                "packs.rpf",
                "packs_mandy3.rpf",
            ],
            "an archive deeper than a pack's folder is not one the game mounts"
        );
    }

    #[test]
    fn a_ps5_shaped_install_lists_a_concat_right_after_the_first_of_its_pair() {
        // ⚠ `levels0.rpf` sorts between the two files of the pair by path.
        let dir = tree(&[
            "levels.rpf",
            "levels0.rpf",
            "levels_concat.rpf",
            "audio_concat.rpf",
            "shaders.rpf",
            "pack/story000/story000.rpf",
            "pack/story000/story000_concat.rpf",
            "pack/story000/story000_script_concat.rpf",
            "pack/zq/shaders_concat.rpf",
        ]);
        let root = dir.path();
        let files = list(root);
        let (story, zq) = (Some("story000"), Some("zq"));
        assert_eq!(
            rows(root, &files),
            [
                row("audio_concat.rpf", Role::Concat, "audio", None),
                row("levels.rpf", Role::First, "levels", None),
                row("levels_concat.rpf", Role::Concat, "levels", None),
                row("levels0.rpf", Role::Alone, "levels0", None),
                row("pack/story000/story000.rpf", Role::First, "story000", story),
                row(
                    "pack/story000/story000_concat.rpf",
                    Role::Concat,
                    "story000",
                    story
                ),
                row(
                    "pack/story000/story000_script_concat.rpf",
                    Role::Concat,
                    "story000_script",
                    story
                ),
                row("pack/zq/shaders_concat.rpf", Role::Concat, "shaders", zq),
                row("shaders.rpf", Role::Alone, "shaders", None),
            ],
            "a `_concat` pairs with the archive of its stem in its own folder alone"
        );
        assert_eq!(mounted(root), files.archive_paths());
    }

    #[test]
    fn a_walk_past_the_most_archives_says_it_was_truncated() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..=MOST_ARCHIVES {
            std::fs::write(dir.path().join(format!("{i:04}.rpf")), []).expect("written");
        }
        let files = list(dir.path());
        assert_eq!(files.archives.len(), MOST_ARCHIVES);
        assert!(files.truncated, "the archive past the limit is said");
        assert_eq!(files.unread, 0);
    }

    #[test]
    fn a_folder_that_does_not_read_is_counted_and_is_no_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = list(&dir.path().join("absent"));
        assert_eq!((files.unread, files.truncated), (1, false));
        assert!(files.archives.is_empty() && files.texture_packs.is_empty());
    }
}
