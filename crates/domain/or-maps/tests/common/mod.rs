//! What the suites of this crate share: the mount of an install, the deletion of one entity of a map with its chain, and the census
//! of the map files a first edit refuses.

#![allow(dead_code)]

pub mod census;
pub mod chain;

use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_title::game::Game;
use or_title::{build_mounted_game, DlcCutoff, GtaInstall, MountedGame};

/// The install mounted with every pack its content list names, none when it cannot be.
pub fn mount(root: &Path) -> Option<MountedGame> {
    let exe = Game::detect_from_root(root).find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    build_mounted_game(&GtaInstall::from_path(root)?, crypto, DlcCutoff::All).ok()
}
