//! What each mode's world-state data switches on at rest: the selectors of the game-shape archives
//! its packs register, found by a scan of each archive's span with the layer manifest beside
//! them, and from them the hours each group is on under each set of modes.
//!
//! ⚠ With no key snapshot no table names a game-shape archive, so no group is on at rest.

use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Rpf9BodyKind, Rpf9Content, Rpf9Scan, Rpf9ZlibRun};
use or_io::gta6::world_state::{
    layers_loaded, picks_with, words_of, Assumed, Place, Request, Selector, WorldState,
    WorldStateError, LAYER_MANIFEST,
};
use or_io::hash::string_hash;
use or_job::Cancel;
use or_title::gta6_mount;

use super::mount::Mount;
use super::{GroupRow, ALL_HOURS};

/// The inflated bytes kept of a body: more than a world-state body was measured to hold.
const KEEP: usize = 32 << 20;

/// A selector of a game-shape layer: its pack's mode bit, the layer it sits in, and whether the level loads that layer itself.
#[derive(Debug, Clone)]
pub(super) struct Placed {
    pub mode: u8,
    /// The hash of the archive's name and of the layer's, as a layer change names them.
    pub layer: (u32, u32),
    pub at_start: bool,
    pub selector: Selector,
}

/// Every selector of the game-shape archives the packs register, with its layer, the places their nodes name, and whether each such archive opened; `None` once `cancel` is set.
pub(super) fn selectors(
    mounted: &Mount,
    every: &[PathBuf],
    cancel: &Cancel,
) -> Option<(Vec<Placed>, Vec<Place>, bool)> {
    let listed = mounted.listed(every, every);
    let (mut found, mut places, mut opened) = (Vec::new(), Vec::new(), true);
    for (mode, archive, entry, registered) in
        gta6_mount::game_shapes(&mounted.registry.packs, &listed)
    {
        let Some(path) = every.get(archive as usize) else {
            continue;
        };
        let stem = registered.rsplit('/').next().unwrap_or(&registered);
        let name = string_hash(stem.as_bytes());
        let span = (entry.offset, entry.offset.saturating_add(entry.stored));
        match bodies(path, span, cancel) {
            Ok(Some((states, layers))) => {
                if layers.is_none() {
                    tracing::warn!(path = %path.display(), "a game-shape archive's scan found no layer manifest: every layer counts as loaded");
                }
                places.extend(states.iter().flat_map(|s| s.places.iter().cloned()));
                found.extend(placed(states, layers.as_deref(), mode.bit(), name))
            }
            Ok(None) => return None,
            Err(error) => {
                opened = false;
                tracing::warn!(path = %path.display(), %error, "a game-shape archive was not read")
            }
        }
    }
    Some((found, places, opened))
}

/// What a scan of a game-shape archive finds: its world-state bodies, and its layer manifest's layers when it finds one.
type Found = (Vec<WorldState>, Option<Vec<(u32, bool)>>);

/// The world-state bodies and the layer manifest a scan of `from..to` of the archive at `path` finds; `None` once `cancel` is set.
fn bodies(path: &Path, (from, to): (u64, u64), cancel: &Cancel) -> io::Result<Option<Found>> {
    let file = File::open(path)?;
    let mut scan = (Rpf9Scan::new(&file, from..to).map_err(io::Error::other)?).keeping(KEEP);
    let (mut found, mut manifest) = (Vec::new(), None);
    while let Some(find) = scan.next() {
        if cancel.is_set() {
            return Ok(None);
        }
        // A scan that fails ends: the rest of the span is not read.
        let find = match find {
            Ok(find) => find,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "the scan of a game-shape archive stopped");
                break;
            }
        };
        let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            content: Rpf9Content::Pso,
            ..
        }) = find.kind
        else {
            continue;
        };
        match WorldState::open(scan.kept(&find)) {
            Ok(state) => found.push(state),
            // The manifest is a body of the same archive, of its own root.
            Err(WorldStateError::NotWorldState(LAYER_MANIFEST)) => {
                match layers_loaded(scan.kept(&find)) {
                    Ok(layers) => manifest = Some(layers),
                    Err(error) => {
                        tracing::warn!(offset = find.offset, %error, "a layer manifest was not read")
                    }
                }
            }
            Err(WorldStateError::NotWorldState(_)) => {}
            Err(error) => {
                tracing::debug!(offset = find.offset, %error, "a world-state body was not read")
            }
        }
    }
    Ok(Some((found, manifest)))
}

/// The selectors of `states` with their layers in the archive named `archive`: a layer `layers` states the level loads, or one it does not name, loads at the start.
fn placed(
    states: Vec<WorldState>,
    layers: Option<&[(u32, bool)]>,
    mode: u8,
    archive: u32,
) -> impl Iterator<Item = Placed> + '_ {
    states.into_iter().flat_map(move |state| {
        let at_start = layers.is_none_or(|l| !l.contains(&(state.layer, false)));
        (state.selectors.into_iter()).map(move |selector| Placed {
            mode,
            layer: (archive, state.layer),
            at_start,
            selector,
        })
    })
}

/// What a fresh game is taken to have that the data does not state, as a person or a study writes it beside the kept index.
#[derive(Debug, Default, serde::Deserialize)]
struct FreshGame {
    /// No flag of the game's flag set is set.
    #[serde(default)]
    flags_clear: bool,
    #[serde(default)]
    requests: Vec<Asked>,
}

/// A request a script makes at a fresh game: keys in hexadecimal, and the hours from and to, both inside, past midnight where the first is not earlier; every hour where none.
#[derive(Debug, serde::Deserialize)]
struct Asked {
    selector: String,
    config: String,
    priority: i32,
    #[serde(default)]
    hours: Option<[u32; 2]>,
}

/// The mask of the hours `from` to `to`, both inside, round midnight where `from` is not earlier.
fn hours_mask([from, to]: [u32; 2]) -> u32 {
    (0..24)
        .filter(|h| match from < to {
            true => from <= *h && *h <= to,
            false => *h <= to || from <= *h,
        })
        .fold(0, |mask, h| mask | 1 << h)
}

/// What the file at `path` takes of a fresh game, and a word of its bytes that a kept index states; nothing and 0 where there is no file.
pub(super) fn assumed(path: &Path) -> (Assumed, u64) {
    let Ok(bytes) = std::fs::read(path) else {
        return (Assumed::default(), 0);
    };
    let word = or_io::gta6::hash::name_hash64(&bytes) | 1;
    let fresh: FreshGame = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        tracing::warn!(path = %path.display(), %error, "what a fresh game is taken to have was not read");
        FreshGame::default()
    });
    let key = |text: &str| u32::from_str_radix(text.trim_start_matches("0x"), 16).ok();
    let requests = (fresh.requests.iter())
        .filter_map(|asked| {
            Some(Request {
                selector: key(&asked.selector)?,
                config: key(&asked.config)?,
                priority: asked.priority,
                hours: asked.hours.map_or(ALL_HOURS, hours_mask),
            })
        })
        .collect();
    let flags = fresh.flags_clear.then(HashSet::new);
    (Assumed { requests, flags }, word)
}

/// The group words on at the whole hour `hour` under the selectors of `placed`, with what `assumed` takes of the game: those of the layers the level loads, then of every layer a picked configuration loads, until none is added.
pub(super) fn words_at(placed: &[&Placed], hour: u32, assumed: &Assumed) -> HashSet<u64> {
    let mut loaded: HashSet<(u32, u32)> = (placed.iter())
        .filter(|p| p.at_start)
        .map(|p| p.layer)
        .collect();
    loop {
        let active: Vec<&Selector> = (placed.iter())
            .filter(|p| loaded.contains(&p.layer))
            .map(|p| &p.selector)
            .collect();
        let configs: Vec<_> = (picks_with(&active, hour, assumed).into_iter())
            .flatten()
            .collect();
        let before = loaded.len();
        loaded.extend(configs.iter().flat_map(|c| c.layers.iter().copied()));
        if loaded.len() == before {
            return words_of(configs);
        }
    }
}

/// Each group of `words` (its id and word, as its maps state them) with the hours it is on under each set of modes, by the selectors of `placed` alone and with what `assumed` takes of a fresh game.
pub(super) fn switches(
    words: &[(u32, u64)],
    placed: &[Placed],
    assumed: &Assumed,
) -> Vec<GroupRow> {
    let mut rows: Vec<GroupRow> = (words.iter())
        .map(|&(id, word)| GroupRow {
            word,
            id,
            pad: 0,
            hours: [0; 8],
            inferred: [0; 8],
        })
        .collect();
    rows.sort_unstable_by_key(|r| (r.id, r.word));
    rows.dedup_by_key(|r| r.id);
    let proven = Assumed::default();
    for modes in 0..8u8 {
        let of_modes: Vec<&Placed> = placed.iter().filter(|p| p.mode & modes != 0).collect();
        for hour in 0..24 {
            let on = words_at(&of_modes, hour, &proven);
            let taken = match assumed == &proven {
                true => on.clone(),
                false => words_at(&of_modes, hour, assumed),
            };
            for row in rows.iter_mut() {
                row.hours[usize::from(modes)] |= u32::from(on.contains(&row.word)) << hour;
                row.inferred[usize::from(modes)] |= u32::from(taken.contains(&row.word)) << hour;
            }
        }
    }
    rows
}

#[cfg(test)]
#[path = "index_states_tests.rs"]
pub(super) mod tests;
