//! The titles whose world this build opens, listed once, and a registered game mapped onto the
//! world core's id of it: the core has no edge to `or_title`, so this crate is where the two meet.

use std::path::Path;

use or_title::Game;
use or_world::contract::{EditTitle, GameId, WorldTitle};

/// One title of the registry.
#[derive(Clone, Copy)]
pub struct Entry {
    pub title: &'static dyn WorldTitle,
    /// Its edit store; `None` where its edits do not go through the contract yet.
    pub edits: Option<&'static dyn EditTitle>,
}

/// Every title, in the order they are asked.
pub fn titles() -> [Entry; 1] {
    [Entry {
        title: &or_world_gta5::Gta5Title,
        edits: None,
    }]
}

/// The title that opens a world of `game`, with the core's id of the game.
pub fn of_game(game: Game) -> Option<(Entry, GameId)> {
    let id = game.profile().id();
    titles().into_iter().find_map(|entry| {
        let games = entry.title.games().iter();
        Some((entry, *games.into_iter().find(|listed| listed.0 == id)?))
    })
}

/// The title that opens the install at `root`, by the game the folder is detected as; a folder no profile claims is asked of GTA V, whose mount refuses one with no `common.rpf`.
pub fn of_install(root: &Path) -> Option<(Entry, GameId)> {
    of_game(Game::detect_from_root(root).unwrap_or(Game::GtaVLegacy))
}

/// The title a kept file or a person names `id`.
pub fn by_id(id: &str) -> Option<Entry> {
    titles().into_iter().find(|entry| entry.title.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_a_title_answers_for_is_registered_and_finds_its_title() {
        for entry in titles() {
            for listed in entry.title.games() {
                let game = Game::by_id(listed.0);
                let game = game.unwrap_or_else(|| panic!("no profile registers `{}`", listed.0));
                let (found, id) = of_game(game).expect("a listed game finds a title");
                assert_eq!((found.title.id(), id), (entry.title.id(), *listed));
            }
        }
    }

    #[test]
    fn no_two_titles_share_an_id_or_a_game() {
        let (mut ids, mut games): (Vec<&str>, Vec<GameId>) = (Vec::new(), Vec::new());
        for entry in titles() {
            ids.push(entry.title.id());
            games.extend(entry.title.games().iter().copied());
        }
        let (all_ids, all_games) = (ids.len(), games.len());
        ids.sort_unstable();
        ids.dedup();
        games.sort_unstable();
        games.dedup();
        assert_eq!((ids.len(), games.len()), (all_ids, all_games));
    }

    #[test]
    fn a_game_no_title_opens_a_world_of_finds_none() {
        assert!(of_game(Game::Rdr2).is_none());
        assert!(by_id("no-such-title").is_none());
        assert!(by_id(or_world_gta5::TITLE).is_some());
    }
}
