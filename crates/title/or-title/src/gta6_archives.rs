//! Bounded discovery of archive paths, without reading archive contents.

use std::io;
use std::path::{Path, PathBuf};

/// Archive files below an install folder, with textures.rpf first and without following symlinks.
pub fn paths(root: &Path, stop: &dyn Fn() -> bool) -> io::Result<Vec<PathBuf>> {
    let mut folders = vec![(root.to_path_buf(), 0)];
    let mut found = Vec::new();
    let mut visited = 0;
    while let Some((folder, depth)) = folders.pop() {
        if stop() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        visited += 1;
        if visited > 10_000 {
            return Err(io::Error::other("archive folder limit exceeded"));
        }
        for entry in std::fs::read_dir(folder)? {
            if stop() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() && depth < 4 {
                folders.push((path, depth + 1));
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("rpf"))
            {
                found.push(path);
                if found.len() > 8192 {
                    return Err(io::Error::other("archive file limit exceeded"));
                }
            }
        }
    }
    found.sort_by_key(|p| {
        (
            !p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("textures.rpf")),
            p.clone(),
        )
    });
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_uses_file_names_and_never_reads_archive_bodies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pack")).unwrap();
        for file in ["data.rpf", "textures.rpf", "pack/extra.RPF", "ignore.txt"] {
            std::fs::write(dir.path().join(file), []).unwrap();
        }
        assert_eq!(
            paths(dir.path(), &|| false).unwrap(),
            ["textures.rpf", "data.rpf", "pack/extra.RPF"].map(|p| dir.path().join(p))
        );
        assert_eq!(
            paths(dir.path(), &|| true).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovery_does_not_follow_symbolic_links() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();
        std::os::unix::fs::symlink("missing", dir.path().join("linked.rpf")).unwrap();
        assert!(paths(dir.path(), &|| false).unwrap().is_empty());
    }
}
