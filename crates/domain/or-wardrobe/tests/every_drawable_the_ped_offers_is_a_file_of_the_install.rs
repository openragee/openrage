//! Every drawable of its own that the preview offers a freemode ped is a model and a texture the install holds.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
//!
//! ⚠ The files are looked up by the names the preview reads them under, in the ped's folder of the
//! install, so a drawable named for a file that is not there would be left out of the picture with a note.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::decl::File;
use or_io::meta::CPedVariationInfo;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::freemode;
use or_title::{build_mounted_game, DlcCutoff, GtaInstall, MountedGame};
use or_wardrobe::dress::{dress, BaseEdit, BaseOutfit, By, Choice, Tone};
use or_wardrobe::{Component, Ped, Wardrobe};

const WHAT: &str = "every_drawable_the_ped_offers_is_a_file_of_the_install";

fn mount(root: &Path) -> Option<MountedGame> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    build_mounted_game(&GtaInstall::from_path(root)?, crypto, DlcCutoff::BaseGame).ok()
}

#[test]
fn every_drawable_the_peds_own_menu_offers_is_a_model_and_a_texture_the_install_holds() {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let Some(game) = mount(&root) else {
        skip_install(WHAT, "the install could not be mounted");
        return;
    };
    let wardrobe = Wardrobe::new("demo");
    let (mut walked, mut missing) = (0usize, Vec::new());
    for ped in Ped::ALL {
        let bytes = game
            .vfs
            .open(&freemode::variation_path(ped.model()))
            .unwrap_or_else(|| panic!("{} has metadata in the install", ped.model()));
        let info = CPedVariationInfo::from_bytes(&bytes).expect("the ped's metadata opens");
        let base = BaseOutfit::of(&info);
        // One listing of the ped's folder: a lookup for each of thousands of names costs minutes.
        let folder = format!("{}/{}", freemode::PED_ARCHIVE, ped.model());
        let held: HashSet<String> = (game.vfs.list(&folder).into_iter())
            .map(|name| name.to_ascii_lowercase())
            .collect();
        assert!(
            !held.is_empty(),
            "{} has a folder in the install",
            ped.model()
        );
        assert!(
            base.drawables(Component::Head) > 2 * Tone::ALL.len(),
            "{} offers heads beyond the base head of each tone",
            ped.model()
        );
        for tone in Tone::ALL {
            let mut choice = Choice::new(ped);
            choice.tone = tone;
            let mut asks = vec![(None, Component::Head)];
            for slot in base.slots() {
                asks.extend((0..base.drawables(slot) as u32).map(|n| (Some(n), slot)));
            }
            for (drawable, slot) in asks {
                choice.base.clear();
                if let Some(n) = drawable {
                    choice.base.insert(slot, BaseEdit::Drawable(n));
                }
                let dressing = dress(&wardrobe, &base, &choice);
                let Some(By::Base { model, texture }) = dressing.slot(slot).map(|w| &w.by) else {
                    panic!("{} wears its own {slot:?}: {dressing:?}", ped.model());
                };
                for file in std::iter::once(model).chain(texture) {
                    walked += 1;
                    if !held.contains(&file.to_ascii_lowercase()) {
                        missing.push(format!(
                            "{}: {file} (asked {slot:?} {drawable:?}, {tone:?})",
                            ped.model()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "{} of {walked} files the menu names are not in the install, the first {:#?}",
        missing.len(),
        &missing[..missing.len().min(5)]
    );
}
