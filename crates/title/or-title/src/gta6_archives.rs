//! Bounded discovery of archive paths, without reading archive contents.

use std::io;
use std::path::{Path, PathBuf};

use or_io::ErrorMode;

/// The most folders visited and archive paths retained during one discovery.
#[derive(Clone, Copy)]
pub struct Limits {
    pub folders: usize,
    pub archives: usize,
}

/// Archive paths in path order, and whether a limit left part of the tree unvisited.
#[derive(Debug, Default)]
pub struct Discovery {
    pub paths: Vec<PathBuf>,
    pub truncated: bool,
}

/// Discover archive paths without following symlinks, returning partial results only at stated limits.
pub fn paths(
    root: &Path,
    limits: Limits,
    errors: ErrorMode,
    stop: &dyn Fn() -> bool,
) -> io::Result<Discovery> {
    let mut found = Discovery::default();
    let mut folders = vec![root.to_path_buf()];
    let mut visited = 0;
    while let Some(folder) = folders.pop() {
        if stop() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if visited == limits.folders {
            found.truncated = true;
            break;
        }
        visited += 1;
        let Some(entries) = readable(std::fs::read_dir(folder), errors)? else {
            continue;
        };
        for entry in entries {
            if stop() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let Some(entry) = readable(entry, errors)? else {
                continue;
            };
            let Some(kind) = readable(entry.file_type(), errors)? else {
                continue;
            };
            if kind.is_dir() {
                if visited + folders.len() < limits.folders {
                    folders.push(entry.path());
                } else {
                    found.truncated = true;
                }
            } else if kind.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("rpf"))
            {
                if found.paths.len() < limits.archives {
                    found.paths.push(entry.path());
                } else {
                    found.truncated = true;
                }
            }
        }
    }
    found.paths.sort();
    Ok(found)
}

fn readable<T>(result: io::Result<T>, errors: ErrorMode) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(_) if errors == ErrorMode::Lenient => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        folders: 20,
        archives: 20,
    };

    #[test]
    fn discovery_uses_file_names_and_never_reads_archive_bodies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pack")).unwrap();
        for file in ["data.rpf", "textures.rpf", "pack/extra.RPF", "ignore.txt"] {
            std::fs::write(dir.path().join(file), []).unwrap();
        }
        let found = paths(dir.path(), LIMITS, ErrorMode::Strict, &|| false).unwrap();
        assert_eq!(
            found.paths,
            ["data.rpf", "pack/extra.RPF", "textures.rpf"].map(|p| dir.path().join(p))
        );
        assert!(!found.truncated);
    }

    #[test]
    fn nested_folders_are_limited_by_count_not_by_an_unreported_depth() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("one/two/three/four/five/six");
        std::fs::create_dir_all(&deep).unwrap();
        let archive = deep.join("deep.rpf");
        std::fs::write(&archive, []).unwrap();
        let found = paths(dir.path(), LIMITS, ErrorMode::Strict, &|| false).unwrap();
        assert_eq!(found.paths, [archive]);
        assert!(!found.truncated);

        let found = paths(
            dir.path(),
            Limits {
                folders: 4,
                ..LIMITS
            },
            ErrorMode::Strict,
            &|| false,
        )
        .unwrap();
        assert!(found.paths.is_empty());
        assert!(found.truncated);
    }

    #[test]
    fn the_archive_limit_reports_partial_results() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["one.rpf", "two.rpf", "three.rpf"] {
            std::fs::write(dir.path().join(name), []).unwrap();
        }
        let found = paths(
            dir.path(),
            Limits {
                archives: 2,
                ..LIMITS
            },
            ErrorMode::Strict,
            &|| false,
        )
        .unwrap();
        assert_eq!(found.paths.len(), 2);
        assert!(found.truncated);
        assert!(found.paths.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn cancellation_is_checked_during_directory_iteration() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("one.rpf"), []).unwrap();
        let calls = std::cell::Cell::new(0);
        let result = paths(dir.path(), LIMITS, ErrorMode::Strict, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 1
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn callers_choose_whether_unreadable_folders_are_skipped_or_refused() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let found = paths(&missing, LIMITS, ErrorMode::Lenient, &|| false).unwrap();
        assert!(found.paths.is_empty());
        assert!(!found.truncated);
        assert_eq!(
            paths(&missing, LIMITS, ErrorMode::Strict, &|| false)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovery_does_not_follow_symbolic_links() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();
        std::os::unix::fs::symlink("missing", dir.path().join("linked.rpf")).unwrap();
        let found = paths(dir.path(), LIMITS, ErrorMode::Strict, &|| false).unwrap();
        assert!(found.paths.is_empty());
        assert!(!found.truncated);
    }
}
