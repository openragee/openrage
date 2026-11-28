//! Every file the `dlcN.rpf` of a real install's DLC packs hold, asked for by the pack's CRC device.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! ⚠ The install is opened read-only and nothing here writes into one.

use std::collections::BTreeSet;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider, RpfCryptoProvider};
use or_title::vfs::{build_mounted_game, DlcCutoff};
use or_title::{Game, GtaInstall};

const WHAT: &str = "every_file_a_real_dlc_sub_pack_holds_answers_to_the_crc_device";

/// `dlc1.rpf`, `dlc2.rpf` and on, which hold what `dlc.rpf` does not.
fn is_a_sub_pack(leaf: &str) -> bool {
    let lower = leaf.to_ascii_lowercase();
    lower
        .strip_prefix("dlc")
        .and_then(|rest| rest.strip_suffix(".rpf"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[test]
fn every_file_a_real_dlc_sub_pack_holds_answers_to_the_crc_device() {
    let roots = or_test_support::game_roots();
    if roots.is_empty() {
        or_test_support::skip_install(
            WHAT,
            format_args!("set {} to an install root", or_test_support::GAME_ROOT_ENV),
        );
        return;
    }
    for root in &roots {
        let Some(exe) = Game::detect_from_root(root).and_then(|game| game.find_executable(root))
        else {
            or_test_support::skip_install(
                WHAT,
                format_args!("{} has no executable", root.display()),
            );
            continue;
        };
        let Ok(keys) = extract_keys(&exe, &|_| {}) else {
            or_test_support::skip_install(WHAT, format_args!("no keys from {}", exe.display()));
            continue;
        };
        let crypto: Arc<dyn RpfCryptoProvider> =
            Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
        let game = build_mounted_game(&GtaInstall { root: root.clone() }, crypto, DlcCutoff::All)
            .expect("the install mounts");

        let mut packs = BTreeSet::new();
        let mut files = 0usize;
        let mut missed = Vec::new();
        game.for_each_entry(|facts| {
            let holder = facts.container.rsplit('/').next().unwrap_or("");
            let Some((device, inside)) = facts.logical.split_once(":/") else {
                return;
            };
            if !is_a_sub_pack(holder) {
                return;
            }
            files += 1;
            packs.insert(device.to_string());
            let by_crc = format!("{device}CRC:/{inside}");
            let (plain, crc) = (game.vfs.locate(&facts.logical), game.vfs.locate(&by_crc));
            if crc.is_none() || crc != plain {
                missed.push(by_crc);
            }
        });

        eprintln!(
            "{}: {files} file(s) in the sub-packs of {} pack(s); {} not found by the CRC device",
            root.display(),
            packs.len(),
            missed.len()
        );
        assert!(
            files > 0,
            "no DLC of {} has a dlcN.rpf, so this checked nothing",
            root.display()
        );
        assert!(
            missed.is_empty(),
            "the CRC device does not answer for: {:?}",
            &missed[..missed.len().min(5)]
        );
    }
}
