//! The mounted tables and configured install that can serve a table-backed model.

use super::document::rpf9::{entry, Rpf9TableDocument};
use super::rpf9_texture_index::Root;
use super::{AppState, DocPayload, DocumentId, DocumentKind, RpfNodeBacking};
use or_job::Cancel;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Default)]
pub(super) struct Scope {
    mounted: Vec<Arc<Rpf9TableDocument>>,
    archives: Vec<PathBuf>,
    folders: Vec<PathBuf>,
}

impl Scope {
    pub fn key(&self) -> Vec<PathBuf> {
        self.archives
            .iter()
            .chain(self.mounted.iter().map(|t| &t.path))
            .chain(&self.folders)
            .cloned()
            .collect()
    }

    pub fn table_versions(&self) -> Vec<usize> {
        self.mounted
            .iter()
            .map(|table| Arc::as_ptr(table) as usize)
            .collect()
    }

    pub fn roots(&self, cancel: &Cancel) -> Vec<Root> {
        let mut roots = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for path in &self.archives {
            if seen.insert(path.clone()) {
                roots.push(
                    match self.mounted.iter().find(|table| &table.path == path) {
                        Some(table) => Root::Mounted(Arc::clone(table)),
                        None => Root::File(path.clone()),
                    },
                );
            }
        }
        for folder in &self.folders {
            let path = folder.join("textures.rpf");
            if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file())
                && seen.insert(path.clone())
            {
                roots.push(match self.mounted.iter().find(|table| table.path == path) {
                    Some(table) => Root::Mounted(Arc::clone(table)),
                    None => Root::File(path),
                });
            }
        }
        for table in &self.mounted {
            if seen.insert(table.path.clone()) {
                roots.push(Root::Mounted(Arc::clone(table)));
            }
        }
        for folder in &self.folders {
            if cancel.is_set() {
                break;
            }
            match or_title::gta6_archives::paths(
                folder,
                or_title::gta6_archives::Limits {
                    folders: 10_000,
                    archives: 8192,
                },
                or_io::ErrorMode::Strict,
                &|| cancel.is_set(),
            ) {
                Ok(mut found) => {
                    if found.truncated {
                        tracing::warn!(path = %folder.display(), "RPF9 archive discovery reached its limit");
                    }
                    found.paths.sort_by_key(|path| {
                        (
                            !path
                                .file_name()
                                .is_some_and(|name| name.eq_ignore_ascii_case("textures.rpf")),
                            path.clone(),
                        )
                    });
                    for path in found.paths {
                        if seen.insert(path.clone()) {
                            roots.push(Root::File(path));
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => break,
                Err(error) => {
                    tracing::warn!(path = %folder.display(), %error, "RPF9 archive discovery failed")
                }
            }
        }
        roots
    }
}

impl AppState {
    pub(super) fn rpf9_texture_scope(&self, model: Option<DocumentId>) -> Scope {
        let mut scope = Scope::default();
        if let Some(doc) = model.and_then(|id| self.document(id)) {
            let path = doc
                .rpf9_source
                .as_ref()
                .map(|s| s.path.as_path())
                .or(doc.path.as_deref());
            if let Some(archive) = path.and_then(entry::archive_path) {
                scope.archives.push(archive.to_path_buf());
                if let Some(folder) = archive.parent() {
                    scope.folders.push(folder.to_path_buf());
                }
            }
        }
        if let Some(folder) = self.prefs.game_paths.get(or_title::Game::GtaVI) {
            if !scope.folders.contains(folder) {
                scope.folders.push(folder.clone());
            }
        }
        for doc in &self.documents {
            if matches!(doc.kind, DocumentKind::GameRoot | DocumentKind::Folder) {
                if let Some(path) = &doc.path {
                    if !scope.folders.contains(path) {
                        scope.folders.push(path.clone());
                    }
                }
            }
            match &doc.payload {
                DocPayload::Rpf9Table(table) => scope.mounted.push(Arc::clone(table)),
                DocPayload::Rpf(rpf) => {
                    for node in rpf.session.nodes.values() {
                        if let RpfNodeBacking::Rpf9Archive {
                            table, owner: None, ..
                        } = &node.backing
                        {
                            if !scope.mounted.iter().any(|t| t.path == table.path) {
                                scope.mounted.push(Arc::clone(table));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        scope.mounted.sort_by(|a, b| a.path.cmp(&b.path));
        scope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_priority_belongs_to_the_texture_scope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        for name in ["a.rpf", "textures.rpf", "nested/textures.rpf"] {
            std::fs::write(dir.path().join(name), []).unwrap();
        }
        let scope = Scope {
            folders: vec![dir.path().to_path_buf()],
            ..Default::default()
        };
        let paths: Vec<_> = scope
            .roots(&Cancel::new())
            .into_iter()
            .map(|root| match root {
                Root::File(path) => path,
                Root::Mounted(table) => table.path.clone(),
            })
            .collect();
        assert_eq!(
            paths,
            ["textures.rpf", "nested/textures.rpf", "a.rpf"].map(|name| dir.path().join(name))
        );
    }
}
