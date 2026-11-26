use std::path::{Path, PathBuf};

use super::prefs::{Game, Prefs};

const INSTALL_MARKER_EXES: &[&str] = &[
    "gta5.exe",
    "playgtav.exe",
    "rdr2.exe",
    "game_scarlett_master.exe",
];

/// What a path is, as far as the edit gate is concerned.
///
/// Three states rather than a bool: the overlay is *inside* an install and yet
/// writable, so "in an install?" alone would refuse it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    /// Not in any game install. Always writable.
    Outside,
    /// Inside an install, under its `mods` directory.
    Overlay,
    /// Inside an install, and part of the game as shipped.
    ShippedGame,
}

/// ⚠ **Membership of an install, not a list of filenames.** A basename list
/// was measured and dropped: of 172 `.rpf` files in a Legacy install it matched
/// 27, missing 82 `dlc.rpf` and 4 `dlc1.rpf` under `update/x64/dlcpacks/`
/// (Enhanced: 27 of 169), and its 31 `patchday*.rpf` entries matched nothing
/// in either install. `the_guard_covers_every_archive_in_an_install` holds the
/// coverage total.
///
/// ⚠ **The overlay rule is `or-title`'s, the one the mount reads through.** A
/// second spelling of "mods" deciding what is writable could disagree with the
/// one deciding what is read, and nothing would see it.
pub fn classify(path: &Path, prefs: &Prefs) -> PathClass {
    if path.as_os_str().is_empty() {
        return PathClass::Outside;
    }
    let Some(root) = install_root_of(path, prefs) else {
        return PathClass::Outside;
    };
    if or_title::is_under_overlay(root, path) {
        return PathClass::Overlay;
    }
    PathClass::ShippedGame
}

/// Which game's install `path` is inside: the configured one that holds it, else the one an executable in a folder above it says.
pub fn game_of(path: &Path, prefs: &Prefs) -> Option<Game> {
    Game::all()
        .find(|&game| {
            (prefs.game_paths.get(game)).is_some_and(|root| path.starts_with(root.as_path()))
        })
        .or_else(|| has_install_marker_in_ancestors(path).map(Game::detect_from_root))
}

/// Where the mods overlay would hold its copy of this shipped `.rpf`; `None` for anything else.
pub fn overlay_copy_target(path: &Path, prefs: &Prefs) -> Option<PathBuf> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rpf"))
    {
        return None;
    }
    or_title::vfs::overlay_path_for(install_root_of(path, prefs)?, path)
}

/// Where the install holds what the mods overlay's copy at `path` stands in for; `None` outside the overlay.
pub fn install_original(path: &Path, prefs: &Prefs) -> Option<PathBuf> {
    or_title::vfs::install_path_for(install_root_of(path, prefs)?, path)
}

pub(super) fn install_root_of<'a>(path: &'a Path, prefs: &'a Prefs) -> Option<&'a Path> {
    is_under_known_install(path, prefs).or_else(|| has_install_marker_in_ancestors(path))
}

/// Which configured install this path is inside, if any.
fn is_under_known_install<'a>(path: &Path, prefs: &'a Prefs) -> Option<&'a Path> {
    Game::all().find_map(|g| {
        prefs
            .game_paths
            .get(g)
            .map(|root| root.as_path())
            .filter(|root| path.starts_with(root))
    })
}

/// The nearest ancestor directory holding a game executable, or a title configuration a title's profile claims, if there is one.
fn has_install_marker_in_ancestors(path: &Path) -> Option<&Path> {
    let mut cur = path;
    for _ in 0..32 {
        let parent = match cur.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => return None,
        };
        for marker in INSTALL_MARKER_EXES {
            if parent.join(marker).exists() {
                return Some(parent);
            }
        }
        // ⚠ A console package has no executable on disk; what it carries is a marker only when a profile claims it.
        if or_title::game::registry::detect(parent).is_some() {
            return Some(parent);
        }
        cur = parent;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::prefs::{GamePaths, Prefs};
    use std::path::PathBuf;

    fn prefs_with_gta_v(root: &Path) -> Prefs {
        let mut game_paths = GamePaths::default();
        game_paths.set(or_title::Game::GtaVLegacy, Some(root.to_path_buf()));
        Prefs {
            game_paths,
            ..Default::default()
        }
    }

    #[test]
    fn empty_path_is_not_retail() {
        let prefs = Prefs::default();
        assert!(classify(&PathBuf::new(), &prefs) != PathClass::ShippedGame);
    }

    #[test]
    fn unrelated_path_is_not_retail() {
        let prefs = Prefs::default();
        let p = PathBuf::from("C:/Users/me/Documents/notes.txt");
        assert!(classify(&p, &prefs) != PathClass::ShippedGame);
    }

    #[test]
    fn known_rpf_under_configured_install_is_retail() {
        let root = PathBuf::from("C:/Games/GTAV");
        let prefs = prefs_with_gta_v(&root);
        let target = root.join("update").join("update.rpf");
        assert!(classify(&target, &prefs) == PathClass::ShippedGame);
    }

    #[test]
    fn known_rpf_outside_install_without_marker_is_not_retail() {
        let prefs = Prefs::default();
        let target = PathBuf::from("D:/Backups/update.rpf");
        assert!(classify(&target, &prefs) != PathClass::ShippedGame);
    }

    #[test]
    fn case_insensitive_basename_match() {
        let root = PathBuf::from("C:/Games/GTAV");
        let prefs = prefs_with_gta_v(&root);
        let target = root.join("UPDATE.RPF");
        assert!(classify(&target, &prefs) == PathClass::ShippedGame);
    }

    #[test]
    fn rdr2_basename_recognised() {
        let root = PathBuf::from("C:/Games/RDR2");
        let mut prefs = Prefs::default();
        prefs
            .game_paths
            .set(or_title::Game::Rdr2, Some(root.clone()));
        let target = root.join("data_0.rpf");
        assert!(classify(&target, &prefs) == PathClass::ShippedGame);
    }
}
