//! Every `.ybn` a real install holds, read for its header alone and read whole.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! ⚠ The install is opened read-only and nothing here writes into one.
//!
//! The front of a file is read from the start of its deflate stream, where
//! `dat_decompress_expecting` also tries a payload behind eight opaque bytes and a stored
//! one, so this is the check that no bound of an install is one of those: a root read from
//! the front must be the root the whole file gives, byte for byte.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider, RpfCryptoProvider};
use or_io::decl::{Raw, Resource as _};
use or_io::gta5::bounds::PhBound;
use or_io::gta5::PhBoundAny;
use or_title::vfs::{build_mounted_game, DlcCutoff};
use or_title::{Game, GtaInstall};

const WHAT: &str = "a_real_installs_bounds_read_for_their_header_give_the_box_of_their_whole_file";

/// What a reader of a bound's box asks of a file: the same bytes as `ModelContents` reads.
const FRONT: usize = or_title::vfs::Vfs::BOUND_FRONT;

#[derive(Default)]
struct Tally {
    bounds: usize,
    agree: usize,
    /// Neither read opens it: not the layout of a Legacy bound.
    both_refuse: usize,
    /// Only the whole file opens, which is a bound the front cannot be trusted for.
    front_alone_fails: Vec<String>,
    /// Both open and the roots differ.
    differ: Vec<String>,
    whole_bytes: u64,
    front_bytes: u64,
    whole_time: Duration,
    front_time: Duration,
}

#[test]
fn every_bound_of_an_install_has_the_same_root_from_its_front_as_whole() {
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

        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        game.for_each_entry(|facts| {
            if facts.leaf.to_ascii_lowercase().ends_with(".ybn")
                && seen.insert(facts.logical.clone())
            {
                paths.push(facts.logical.clone());
            }
        });

        let root_len = size_of::<Raw<PhBound>>();
        let mut tally = Tally::default();
        for path in &paths {
            let started = Instant::now();
            let Some(whole) = game.vfs.open(path) else {
                continue;
            };
            let from_whole = PhBoundAny::peek(&whole)
                .ok()
                .map(|peek| peek.image().bytes()[..root_len].to_vec());
            tally.whole_time += started.elapsed();

            let started = Instant::now();
            let Some(front) = game.vfs.open_head(path, FRONT) else {
                continue;
            };
            let from_front = PhBoundAny::peek(&front)
                .ok()
                .map(|peek| peek.image().bytes()[..root_len].to_vec());
            tally.front_time += started.elapsed();

            tally.bounds += 1;
            tally.whole_bytes += whole.len() as u64;
            tally.front_bytes += front.len() as u64;
            match (from_whole, from_front) {
                (Some(a), Some(b)) if a == b => tally.agree += 1,
                (None, None) => tally.both_refuse += 1,
                (Some(_), None) => tally.front_alone_fails.push(path.clone()),
                _ => tally.differ.push(path.clone()),
            }
        }

        eprintln!(
            "{}: {} of {} .ybn read both ways; the same root for {}, neither reads {}, {} read \
             whole alone, {} differ; {:.1} MiB read whole in {:.1} s, {:.1} MiB from the front in {:.1} s",
            root.display(),
            tally.bounds,
            paths.len(),
            tally.agree,
            tally.both_refuse,
            tally.front_alone_fails.len(),
            tally.differ.len(),
            tally.whole_bytes as f64 / (1024.0 * 1024.0),
            tally.whole_time.as_secs_f64(),
            tally.front_bytes as f64 / (1024.0 * 1024.0),
            tally.front_time.as_secs_f64(),
        );
        assert!(
            tally.bounds > 1_000 && tally.agree > 0,
            "{} .ybn is not an install's collision; the walk is wrong, not the game",
            tally.bounds
        );
        assert!(
            tally.front_alone_fails.is_empty() && tally.differ.is_empty(),
            "the front of a bound did not give the root the whole file gives: {:?} {:?}",
            &tally.front_alone_fails[..tally.front_alone_fails.len().min(5)],
            &tally.differ[..tally.differ.len().min(5)]
        );
    }
}
