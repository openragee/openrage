//! What the suites of this crate share: the mount of an install, and a small particle list.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_title::game::Game;
use or_title::{build_mounted_game, DlcCutoff, GtaInstall, MountedGame};

/// The install mounted with every pack its content list names, none when it cannot be.
pub fn mount(root: &Path) -> Option<MountedGame> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    build_mounted_game(&GtaInstall::from_path(root)?, crypto, DlcCutoff::All).ok()
}

/// A `.ypt` with the effect `effect`, firing one emitter rule and one particle rule, and no texture.
pub fn a_ypt(name: &str, effect: &str) -> Vec<u8> {
    or_particle::sample::list_of(name, &[effect])
}
