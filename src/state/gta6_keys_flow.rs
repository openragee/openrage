//! GTA VI's keys in the app: the executable in the GTA VI install folder the user set unmasks the
//! bundled key snapshot, or yields a fresh one, cached masked beside the keys cache.

use std::path::PathBuf;

use or_io::container::rpf::{GtaViKeys, GTA6_EXE_ENV};
use or_title::Game;

#[cfg(test)]
use crate::state::prefs::Prefs;
use crate::state::prefs::{config_root, prefs_path, read_prefs};

/// GTA VI's keys from the user's snapshot, unmasked with `exe`, or the GTA VI executable `exe`; else
/// a sentence that says what to do.
pub(crate) fn load_gta6_keys_with(exe: Option<&std::path::Path>) -> Result<GtaViKeys, String> {
    said(GtaViKeys::for_install_with(config_root().as_deref(), exe))
}

/// The executable that unmasks the keys: [`GTA6_EXE_ENV`], else the one in the install folder set.
pub(crate) fn executable() -> Option<PathBuf> {
    std::env::var_os(GTA6_EXE_ENV)
        .map(PathBuf::from)
        .or_else(configured_executable)
}

/// The executable in the install folder the preferences hold for either build: the Xbox package's
/// `game_scarlett_master.exe`, the PlayStation 5 dump's `eboot.bin`.
///
/// ⚠ Read from the file and not through `load_prefs`, which would reset the base a later save merges
/// against, from a reader's thread.
fn configured_executable() -> Option<PathBuf> {
    let prefs = read_prefs(&prefs_path()?).0;
    [Game::GtaVI, Game::GtaVIPs5]
        .into_iter()
        .find_map(|game| game.find_executable(prefs.game_paths.get(game)?))
}

fn said(found: or_io::Result<Option<GtaViKeys>>) -> Result<GtaViKeys, String> {
    match found {
        Ok(Some(keys)) => Ok(keys),
        Ok(None) => Err(no_keys()),
        Err(e) => Err(e.to_string()),
    }
}

/// Why no keys are held: no install folder is set, or the build set has no executable to read them from.
fn no_keys() -> String {
    let prefs = prefs_path().map(|file| read_prefs(&file).0);
    let install = prefs.as_ref().and_then(crate::state::gta_vi::install);
    match install {
        Some((build, folder)) if build.find_executable(folder).is_none() => tr!(
            "No GTA VI keys: the {platform} build's folder holds no executable OpenRage reads them \
             from, so they come only from a key snapshot of that build.",
            platform = build.profile().platform().label()
        ),
        _ => tr!(
            "No GTA VI keys: set the GTA VI install folder in Settings ▸ Games so its executable can unlock them."
        )
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::prefs::{keep_prefs_in, save_prefs};

    #[test]
    fn the_executable_is_found_in_the_gta_vi_install_folder_set() {
        let config = tempfile::tempdir().expect("a config folder");
        let install = tempfile::tempdir().expect("an install folder");
        keep_prefs_in(config.path());
        assert_eq!(
            configured_executable(),
            None,
            "no folder set, no executable"
        );

        let mut prefs = Prefs::default();
        prefs
            .game_paths
            .set(Game::GtaVI, Some(install.path().to_path_buf()));
        save_prefs(&prefs);
        assert_eq!(
            configured_executable(),
            None,
            "a folder without the executable"
        );

        let exe = install.path().join(Game::GtaVI.executable_name());
        std::fs::write(&exe, b"not an executable").expect("write");
        assert_eq!(configured_executable(), Some(exe.clone()));

        // A file another tool saved with a byte-order mark still names the folder.
        let file = prefs_path().expect("a prefs file");
        let saved = std::fs::read(&file).expect("read");
        std::fs::write(&file, [&[0xEF, 0xBB, 0xBF][..], &saved].concat()).expect("write");
        assert_eq!(configured_executable(), Some(exe));
    }

    #[test]
    fn no_keys_say_where_the_install_folder_is_set() {
        let config = tempfile::tempdir().expect("a config folder");
        keep_prefs_in(config.path());
        let err = said(Ok(None)).err().expect("no keys");
        assert!(err.contains("Settings ▸ Games"), "{err}");

        // A build with no executable is not told to set the folder it already set.
        let mut prefs = Prefs::default();
        let folder = Some(config.path().to_path_buf());
        prefs.game_paths.set(Game::GtaVIPs5, folder);
        save_prefs(&prefs);
        let err = said(Ok(None)).err().expect("no keys");
        let platform = Game::GtaVIPs5.profile().platform().label();
        assert!(
            err.contains(platform) && !err.contains("Settings ▸ Games"),
            "{err}"
        );
    }

    #[test]
    fn an_executable_that_keys_nothing_is_refused_naming_it_and_nothing_is_cached() {
        // A snapshot the environment names is the user's own and wins over any executable.
        if std::env::var_os(or_io::container::rpf::GTA6_SNAPSHOT_ENV).is_some() {
            return;
        }
        let config = tempfile::tempdir().expect("a config folder");
        let install = tempfile::tempdir().expect("an install folder");
        let exe = install.path().join(Game::GtaVI.executable_name());
        std::fs::write(&exe, b"not an executable").expect("write");
        let found = GtaViKeys::for_install_with(Some(config.path()), Some(exe.as_path()));
        let err = said(found).err().expect("it keys nothing");
        assert!(err.contains(&exe.display().to_string()), "{err}");
        let cache = or_io::container::rpf::gta6_extract::KEYS_CACHE_FILE_NAME;
        assert!(!config.path().join(cache).exists());
    }
}
