//! The map files of a mount whose first edit is refused, each with the sentence: what the install suite counts, and what
//! `the_first_edit_of_a_game_object_imports_its_map` holds on an install built by hand where no install is.

use std::collections::BTreeMap;

use or_maps::first_edit::{plan, Change, Edit, Game, Place, Refusal, Which};
use or_project::{Container, Module, Project, Target};
use or_title::MountedGame;

/// The mount as the move of one entity asks it: the bytes of a file and where they lie. A move follows no chain, so no map is found by name.
struct Files<'a>(&'a MountedGame);

impl Game for Files<'_> {
    fn edition(&self) -> Target {
        Target::Legacy
    }

    fn read(&self, logical: &str) -> Option<Vec<u8>> {
        self.0.vfs.open(logical)
    }

    fn lies_at(&self, logical: &str) -> Option<String> {
        self.0.vfs.locate(logical)
    }

    fn map_file(&self, _: u32) -> Option<String> {
        None
    }

    fn maps_under(&self, _: u32) -> Vec<String> {
        Vec::new()
    }
}

/// What the first edit of an object said of the map files of a mount.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Census {
    /// The map files the mount loads, one for each name.
    pub maps: usize,
    /// How many of them were asked.
    pub asked: usize,
    /// The ones asked that hold no entity, which no edit names: what they would be refused for is not known.
    pub empty: usize,
    /// The ones asked whose first edit is refused, by logical path in the order of their names, each with the sentence.
    pub refused: Vec<(String, String)>,
}

impl Census {
    /// The count with its denominator, then a line for each map file refused.
    pub fn text(&self) -> String {
        let mut text = format!(
            "{} of {} map files asked refuse a first edit ({} of the {} the mount loads were asked, and {} of those hold no entity)",
            self.refused.len(),
            self.asked,
            self.asked,
            self.maps,
            self.empty
        );
        for (_, why) in &self.refused {
            text.push_str("\n  ");
            text.push_str(why);
        }
        text
    }
}

/// Plan the move of the first entity of every `stride`th map file `game` loads, over a maps content that holds nothing, and apply none.
pub fn census(game: &MountedGame, stride: usize) -> Census {
    let mut winners: BTreeMap<String, String> = BTreeMap::new();
    game.for_each_file(|logical, _, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if leaf.ends_with(".ymap") {
            winners.insert(leaf, logical.to_string());
        }
    });
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let container = Container {
        modules: vec![Module::maps("maps", "maps")],
        outputs: Vec::new(),
    };
    let file = scratch.path().join("census.openrage-proj");
    Project::create(&file, &container, &[]).expect("a scratch project");
    or_maps::adopt::start_module(scratch.path(), &container.modules[0]).expect("a maps content");
    let place = Place {
        project: &file,
        container: &container,
        module: &container.modules[0],
    };

    let mut census = Census {
        maps: winners.len(),
        ..Census::default()
    };
    for logical in winners.values().step_by(stride.max(1)) {
        census.asked += 1;
        let edit = Edit {
            map: logical.clone(),
            entity: Which::Index(0),
            change: Change::Move { position: [0.0; 3] },
        };
        match plan(&place, &Files(game), &edit) {
            Ok(_) => {}
            Err(Refusal::NoSuchEntity { .. }) => census.empty += 1,
            Err(refused) => census.refused.push((logical.clone(), refused.to_string())),
        }
    }
    census
}
