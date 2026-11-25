//! The world index of a real install, held to everything its mount lists.
//!
//! ⚠ Needs an install: without one the test records a skip and has checked nothing. It reads
//! every map, type file and bound of the install once, which is why it is in the slow tier.

mod world_fixture;

use std::sync::Arc;

use or_title::vfs::{build_mounted_game, DlcCutoff};
use or_title::GtaInstall;
use world_fixture::held_to_its_mount;

const WHAT: &str = "the_world_index_names_everything_a_real_install_lists";

#[test]
fn the_world_index_names_everything_a_real_install_lists() {
    let roots = or_test_support::game_roots();
    let Some(root) = roots.first() else {
        or_test_support::skip_install(
            WHAT,
            format_args!("set {} to an install root", or_test_support::GAME_ROOT_ENV),
        );
        return;
    };
    let exe = or_title::Game::detect_from_root(root).find_executable(root);
    let Some(exe) = exe else {
        or_test_support::skip_install(WHAT, "no executable, so no keys");
        return;
    };
    let Ok(keys) = or_io::container::rpf::crypto::extract_keys(&exe, &|_| {}) else {
        or_test_support::skip_install(WHAT, "key extraction failed");
        return;
    };
    if let Some(path) = or_test_support::schema_snapshot() {
        if let Err(e) = or_io::meta::install_snapshot(&path) {
            panic!(
                "the staged schema snapshot {} is unusable: {e}",
                path.display()
            );
        }
    }
    let crypto: Arc<dyn or_io::container::rpf::crypto::RpfCryptoProvider> =
        Arc::new(or_io::container::rpf::crypto::Gta5RetailCryptoProvider::new(Arc::new(keys)));
    let mount = || {
        build_mounted_game(
            &GtaInstall { root: root.clone() },
            Arc::clone(&crypto),
            DlcCutoff::All,
        )
        .expect("the install mounts")
    };
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let held = held_to_its_mount(&mount, root, &scratch.path().join("world.index"));
    assert!(held.maps > 0 && held.type_files > 0, "{held:?}");
}
