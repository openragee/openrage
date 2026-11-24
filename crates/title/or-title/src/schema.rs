//! Where the editor and the CLI find the metadata schema snapshot: the
//! variable, then the user's configuration directory, then, from a source
//! tree only, the shared store beside the checkout.

use std::path::{Path, PathBuf};

/// Names a snapshot file, or `none` to run with no schema.
pub const SCHEMA_ENV: &str = "OPENRAGE_SCHEMA";

/// The snapshot's file name in the user's configuration directory, beside
/// the keys cache.
pub const CONFIG_SNAPSHOT: &str = "schema_snapshot.bin";

/// The corpus directory the shared store lives in.
pub const STORE_DIR: &str = "openrage-corpus";

/// The shared store's file, under [`STORE_DIR`].
///
/// ⚠ `v3` is `or_io::meta::snapshot_discovery::SNAPSHOT_FORMAT`: a snapshot is
/// `bincode` of the type model, so each shape keeps its own directory and a
/// regenerated file never breaks a tree still on the older shape.
pub const STORE_FILE: &str = "or-meta/v3/schema_snapshot.bin";

/// Names GTA VI's snapshot file, which is read for its names beside the first.
pub const GTA6_SCHEMA_ENV: &str = "OPENRAGE_GTA6_SCHEMA";

/// GTA VI's snapshot in the user's configuration directory.
pub const GTA6_CONFIG_SNAPSHOT: &str = "gta6_schema_snapshot.bin";

/// GTA VI's snapshot in the shared store, of the same shape as [`STORE_FILE`].
pub const GTA6_STORE_FILE: &str = "or-meta/v3/gta6_schema_snapshot.bin";

/// ⚠ A word, not an empty value: PowerShell deletes a variable assigned `""`,
/// so an empty-string convention would silently mean "unset".
pub const NO_SCHEMA: &[&str] = &["none", "off"];

pub fn asks_for_no_schema(value: &str) -> bool {
    NO_SCHEMA.contains(&value.trim().to_ascii_lowercase().as_str())
}

/// Where the store's file goes in a corpus directory.
pub fn store_file(corpus: &Path) -> PathBuf {
    corpus.join(STORE_FILE)
}

/// The source tree this binary was built from, while it is still on disk.
///
/// ⚠ A compile-time path rather than the executable's location: the
/// self-hosted runner builds into a target directory outside the checkout.
pub fn source_tree() -> Option<&'static Path> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.join("Cargo.toml").is_file().then_some(dir)
}

/// The nearest corpus directory above `source_tree`.
///
/// ⚠ Every ancestor, not the checkout's parent: a lane two levels below the
/// organisation folder would otherwise name `openrage-lanes/openrage-corpus`.
pub fn corpus_above(source_tree: &Path) -> Option<PathBuf> {
    source_tree
        .ancestors()
        .map(|dir| dir.join(STORE_DIR))
        .find(|dir| dir.is_dir())
}

/// Every path a snapshot may come from, in the order they are tried; empty
/// when `named` asks for none.
pub fn candidates(
    named: Option<&str>,
    config_dir: Option<&Path>,
    source_tree: Option<&Path>,
) -> Vec<PathBuf> {
    if named.is_some_and(asks_for_no_schema) {
        return Vec::new();
    }
    let mut paths = Vec::new();
    paths.extend(
        named
            .filter(|v| !v.trim().is_empty())
            .map(|v| PathBuf::from(v.trim())),
    );
    paths.extend(config_dir.map(|dir| dir.join(CONFIG_SNAPSHOT)));
    if let Some(tree) = source_tree {
        paths.extend(tree.ancestors().map(|dir| store_file(&dir.join(STORE_DIR))));
    }
    paths
}

/// The snapshot to install, given the user's configuration directory.
pub fn find_snapshot(config_dir: Option<&Path>) -> Option<PathBuf> {
    let named = std::env::var(SCHEMA_ENV).ok();
    candidates(named.as_deref(), config_dir, source_tree())
        .into_iter()
        .find(|p| p.is_file())
}

/// GTA VI's snapshot to install for its names; none where [`SCHEMA_ENV`] asks for no schema.
pub fn find_gta6_snapshot(config_dir: Option<&Path>) -> Option<PathBuf> {
    if std::env::var(SCHEMA_ENV).is_ok_and(|v| asks_for_no_schema(&v)) {
        return None;
    }
    let named = std::env::var(GTA6_SCHEMA_ENV).ok();
    gta6_candidates(named.as_deref(), config_dir, source_tree())
        .into_iter()
        .find(|p| p.is_file())
}

fn gta6_candidates(
    named: Option<&str>,
    config_dir: Option<&Path>,
    source_tree: Option<&Path>,
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = (named.map(str::trim).filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .into_iter()
        .collect();
    paths.extend(config_dir.map(|dir| dir.join(GTA6_CONFIG_SNAPSHOT)));
    if let Some(corpus) = source_tree.and_then(corpus_above) {
        paths.push(corpus.join(GTA6_STORE_FILE));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gta_vi_s_snapshot_is_named_then_configured_and_never_the_first_title_s() {
        let config = PathBuf::from("C:/cfg");
        let paths = gta6_candidates(Some(" D:/x/six.bin "), Some(&config), None);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("D:/x/six.bin"),
                config.join(GTA6_CONFIG_SNAPSHOT)
            ]
        );
        assert_ne!(GTA6_CONFIG_SNAPSHOT, CONFIG_SNAPSHOT);
        assert_ne!(GTA6_STORE_FILE, STORE_FILE);
        assert_eq!(
            Path::new(GTA6_STORE_FILE).parent(),
            Path::new(STORE_FILE).parent(),
            "one shape, one directory"
        );
    }

    fn org() -> PathBuf {
        PathBuf::from("D:/org")
    }

    #[test]
    fn the_variable_then_the_config_directory_then_the_store() {
        let tree = org().join("openrage/crates/title/or-title");
        let config = PathBuf::from("C:/Users/me/AppData/Roaming/openrage");
        let named = "D:/elsewhere/schema_snapshot.bin";
        let paths = candidates(Some(named), Some(&config), Some(&tree));

        assert_eq!(paths[0], PathBuf::from(named));
        assert_eq!(paths[1], config.join(CONFIG_SNAPSHOT));
        let store = org().join("openrage-corpus/or-meta/v3/schema_snapshot.bin");
        assert!(paths[2..].contains(&store), "got {paths:?}");
    }

    /// ⚠ A lane lives at `<org>/openrage-lanes/<lane>`, so the checkout's
    /// parent is the wrong spelling; the walk has to reach the organisation
    /// folder, and the lane and the checkout have to agree on the file.
    #[test]
    fn a_lane_and_the_checkout_reach_the_same_store() {
        let store = org().join("openrage-corpus/or-meta/v3/schema_snapshot.bin");
        let lane = org().join("openrage-lanes/mylane/crates/title/or-title");
        let checkout = org().join("openrage/crates/title/or-title");
        assert!(candidates(None, None, Some(&lane)).contains(&store));
        assert!(candidates(None, None, Some(&checkout)).contains(&store));
    }

    /// Outside a source tree only the variable and the configuration
    /// directory are searched.
    #[test]
    fn a_release_does_not_walk_for_the_store() {
        let config = PathBuf::from("C:/cfg");
        assert_eq!(
            candidates(None, Some(&config), None),
            vec![config.join(CONFIG_SNAPSHOT)]
        );
    }

    #[test]
    fn only_the_sentinel_asks_for_no_schema() {
        for sentinel in NO_SCHEMA {
            assert!(asks_for_no_schema(sentinel));
        }
        assert!(asks_for_no_schema("  NoNe "));
        assert!(
            !asks_for_no_schema(""),
            "PowerShell deletes a variable assigned an empty string"
        );
        assert!(!asks_for_no_schema(
            "D:/snapshots/none-of-these/schema_snapshot.bin"
        ));
        assert!(candidates(Some("none"), Some(Path::new("C:/cfg")), Some(&org())).is_empty());
    }

    #[test]
    fn the_store_is_kept_under_its_shape() {
        assert!(STORE_FILE.ends_with("/schema_snapshot.bin"));
        assert!(Path::new(STORE_FILE)
            .parent()
            .is_some_and(|p| p.ends_with("v3")));
        assert_eq!(
            store_file(&org().join(STORE_DIR)),
            org().join("openrage-corpus/or-meta/v3/schema_snapshot.bin")
        );
    }
}
