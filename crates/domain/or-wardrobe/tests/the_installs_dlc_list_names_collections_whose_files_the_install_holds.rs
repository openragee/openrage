//! The DLC list a pack that replaces the game's files chooses from names collections the install really holds.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
//!
//! ⚠ Each collection is read from the `dlcName` of a shop metadata file a DLC pack registers, and this holds it to the
//! folders of the install: a DLC archive keeps the files of a collection in a folder named for the ped and the collection.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::freemode::dlc_collections;
use or_title::{build_mounted_game, DlcCutoff, GtaInstall, MountedGame};

const WHAT: &str = "the_installs_dlc_list_names_collections_whose_files_the_install_holds";

fn mount(root: &Path) -> Option<MountedGame> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    build_mounted_game(&GtaInstall::from_path(root)?, crypto, DlcCutoff::All).ok()
}

#[test]
fn every_collection_the_dlc_list_names_has_a_folder_of_that_ped_and_collection_in_the_install() {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let Some(game) = mount(&root) else {
        skip_install(WHAT, "the install could not be mounted");
        return;
    };
    let listed = dlc_collections(&game);
    assert!(
        !listed.is_empty(),
        "the install's DLC packs register collections for the freemode peds"
    );

    // One walk of the install's entry tables, keeping each folder named for a freemode ped.
    let mut folders: BTreeSet<String> = BTreeSet::new();
    game.for_each_file(|logical, _, _| {
        for part in logical.split(['/', '\\']) {
            let part = part.to_ascii_lowercase();
            if part.starts_with("mp_m_freemode_01_") || part.starts_with("mp_f_freemode_01_") {
                folders.insert(part);
            }
        }
    });
    let missing: Vec<String> = listed
        .iter()
        .filter(|c| {
            // The props of a collection are in a folder of their own, `<ped>_p_<collection>`.
            let (comps, props) = (
                format!("{}_{}", c.ped, c.collection),
                format!("{}_p_{}", c.ped, c.collection),
            );
            !folders.contains(&comps) && !folders.contains(&props)
        })
        .map(|c| format!("{} {} of {}", c.ped, c.collection, c.pack))
        .collect();
    println!(
        "{} collection(s) listed from {} DLC pack(s); {} with no folder of its name: {missing:?}",
        listed.len(),
        listed
            .iter()
            .map(|c| &c.pack)
            .collect::<BTreeSet<_>>()
            .len(),
        missing.len()
    );
    assert!(
        missing.is_empty(),
        "a collection the list names should have a folder in the install: {missing:?}"
    );
}
