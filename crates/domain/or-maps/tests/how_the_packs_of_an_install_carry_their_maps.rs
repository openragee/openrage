//! How the packs of a mounted install carry their maps: the archive that holds the map files, what the pack's content document states
//! of it and which change set turns it on, where the type files sit and how they are registered, what shares the maps' archive, and
//! where the manifest of the maps sits. It is the measurement the layout of a story pack is built from (`or_maps::layout`), and the
//! one that says what still locks the cell of a singleplayer DLC for maps (`or_project::cannot_ship`).
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
//!
//! ⚠ The install is opened read-only and nothing here writes into one. It prints counts, and never a name of the game.

mod common;

use common::packs::{departures, kind_of, measure, report, AS_AN_UPDATE, FOR_MULTIPLAYER};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::dlc::SetupContentType;

const WHAT: &str = "how_the_packs_of_an_install_carry_their_maps";

#[test]
fn a_pack_keeps_its_maps_in_an_archive_of_the_level_stated_as_map_data_and_asks_for_its_type_files_from_archives_of_models(
) {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let game = common::mount(&root).expect("the install mounts");
    let figures = measure(&game);
    report(&figures);
    let f = &figures;

    // Enough was read for the figures to mean something.
    assert!(f.packs_with_maps >= 20 && f.registered >= 100 && f.asked >= 100);
    assert!(f.interiors >= 100 && f.asked_unread == 0);
    // ⚠ The pattern the layout of a story pack is built from, with no exception; among it, what keeps the cell of a singleplayer DLC
    // locked for maps: no pack turns an archive of maps on when it starts, or for the story.
    assert_eq!(departures(f), Vec::<&str>::new());
    // Who turns an archive of maps on: a compat pack from the map group of multiplayer, a level pack from the group its update
    // streams from, and a few from a change set no group names.
    for (by, kind) in f.turned_on_in.keys() {
        let expected = match by {
            by if by.starts_with(FOR_MULTIPLAYER) => SetupContentType::CompatPack,
            by if by.starts_with(AS_AN_UPDATE) => SetupContentType::LevelPack,
            by => {
                assert!(by.starts_with("no group"), "{by}");
                continue;
            }
        };
        assert_eq!(*kind, kind_of(expected), "{by}");
    }
    // Some archives of maps sit in the folder of the level directly, which is where a build writes its own.
    assert!(f.depth.get(&0).copied().unwrap_or(0) > 0);
    // A model and a map share an archive in few of them, and a type file asked for is under the props folder in most: the two
    // places a build keeps apart.
    assert!(f.with_models * 10 < f.map_archives);
    assert!(f.asked_under_props * 10 > f.asked * 9);
    // ⚠ A manifest names some of the type files a pack asks for as needed and not others, so being asked for says nothing of
    // whether the manifest of a build's maps names a type file.
    assert!(0 < f.asked_named_by_a_manifest && f.asked_named_by_a_manifest < f.asked);
}
