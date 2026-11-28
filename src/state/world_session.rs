//! The World screen's session: one world of one game's install, opened by the game the active
//! install is of, behind a seam the screen, its panel and the render bridge read it through.
//!
//! ⚠ A session is keyed by its game, its install and its options: another key drops the session
//! held and opens another, and two sessions share no state.
//!
//! ⚠ GTA VI's session is a shell over its viewer, which is still one value for the process: its
//! screen and its bridge reach that viewer themselves, and only what is asked here goes through
//! the seam.

use std::path::PathBuf;
use std::sync::Arc;

use or_renderer::FlyCamera;
use or_world::caps::WorldCaps;
use or_world::model::{AssetKey, FrameFacts, Instances, Texture, TextureAsk, WaterQuad};
use or_world::sentence::Sentence;
use or_world::slots::{OptionId, OptionKind, OptionValue, WeatherKind};
use or_world::streaming::NodeState;

use super::prefs::{Game, Prefs};
use super::AppState;

pub mod driven;
pub mod gta6;
pub mod mesh;
#[cfg(test)]
mod tests;

pub use mesh::ArrivedModel;

/// What a session is of: all of it keys the session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorldKey {
    pub game: Game,
    pub install: PathBuf,
    /// What the title lets a person choose at open, each by its id, sorted.
    pub options: Vec<(OptionId, OptionValue)>,
}

/// Where a session stands between its opening and its world.
#[derive(Debug, Clone, PartialEq)]
pub enum Standing {
    /// Its index is being taken: how far, from 0 to 1.
    Opening(f32),
    /// It did not open, and why.
    Failed(String),
    Open,
}

/// What a world still waits for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Waiting {
    pub maps: usize,
    pub models: usize,
    pub textures: usize,
}

impl Waiting {
    pub fn any(&self) -> bool {
        self.maps + self.models + self.textures > 0
    }
}

/// What a selection holds that the plain pass does not draw.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Undrawn {
    /// The instances of a thing nothing says what draws, which would stand as a box.
    pub stand_ins: usize,
    /// The lists of instances of the maps loaded that are not laid out.
    pub lists: usize,
}

/// What a thing that could not be read is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Map,
    Model,
    Texture,
}

/// Why a thing could not be read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// A sentence the catalogs hold.
    Said(Sentence),
    /// What the reader said, in its own words.
    Told(String),
}

impl Reason {
    /// The reason in the language in use.
    pub fn shown(&self) -> String {
        match self {
            Self::Said(sentence) => crate::i18n::tr(sentence.key()).to_string(),
            Self::Told(text) => text.clone(),
        }
    }
}

/// How many things of one kind could not be read for one reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unread {
    pub kind: Kind,
    pub why: Reason,
    pub count: usize,
}

/// What a frame draws of a world: the instances a selection laid out, each naming its model.
#[derive(Clone, Default)]
pub struct Selection {
    /// Counted up whenever the instances changed.
    pub version: u64,
    pub instances: Instances,
    /// Whether a fade in time is under way.
    pub fading: bool,
}

/// What a session gives a frame, and what the frame tells it back: models and textures as plain data, with no type of a game.
pub trait WorldFeed {
    /// What tells this session from any other the device held data of.
    fn id(&self) -> u64;
    fn camera(&self) -> FlyCamera;
    /// The look's frame values at the session's hour.
    fn facts(&self) -> FrameFacts;
    /// The last selection, where one was made.
    fn selection(&self) -> Option<Arc<Selection>>;
    /// A model that arrived, or `None` for one that never will.
    fn take_model(&mut self) -> Option<(AssetKey, Option<ArrivedModel>)>;
    /// A texture that arrived, or `None` for one that never will.
    fn take_texture(&mut self) -> Option<(TextureAsk, Option<Texture>)>;
    /// Where `model` stands on the device: there, never to come, or let go and to be read again.
    fn tell(&mut self, model: AssetKey, stands: NodeState);
    /// The textures the frame waits for, each with how soon it is needed, in place of those asked before.
    fn want_textures(&mut self, asks: Vec<(f32, TextureAsk)>);
    /// The world's sheets of water, once read; `None` until then and for a world that states none.
    fn water(&self) -> Option<Arc<[WaterQuad]>> {
        None
    }
}

/// One world session, as the World screen, its panel and the bridge read it.
pub trait WorldSession {
    fn key(&self) -> &WorldKey;
    /// Take what the session's threads sent: called once a frame.
    fn poll(&mut self);
    fn standing(&self) -> Standing;
    /// What the world states it has; none of them until it is open.
    fn caps(&self) -> WorldCaps;
    /// What its title lets a person choose at open.
    fn option_kinds(&self) -> Vec<OptionKind>;
    fn camera(&self) -> FlyCamera;
    fn set_camera(&mut self, camera: FlyCamera);
    /// The hour of the day, from 0 and below 24.
    fn hour(&self) -> f32;
    fn set_hour(&mut self, hour: f32);
    /// The weathers the world is looked at under, the one it opened under first; none where its title names no weather.
    fn weathers(&self) -> Vec<WeatherKind> {
        Vec::new()
    }
    /// The weather it is looked at under, by the title's name of it.
    fn weather(&self) -> u64 {
        0
    }
    fn set_weather(&mut self, _weather: u64) {}
    /// What the last selection holds that is not drawn.
    fn undrawn(&self) -> Undrawn {
        Undrawn::default()
    }
    fn waiting(&self) -> Waiting;
    /// What could not be read, each kind and reason once, the most first.
    fn unread(&self) -> Vec<Unread>;
    /// What the bridge's neutral path draws; `None` where the session draws through a path of its own.
    fn feed(&mut self) -> Option<&mut dyn WorldFeed>;
}

/// The session the World screen holds, and how many it has opened.
#[derive(Default)]
pub struct Held {
    session: Option<Box<dyn WorldSession>>,
    /// What a person chose at open for each game, kept while the app runs.
    pub options: Vec<(Game, Vec<(OptionId, OptionValue)>)>,
    pub opened: u64,
}

impl Held {
    /// The key of the session held.
    pub fn key(&self) -> Option<&WorldKey> {
        self.session.as_deref().map(WorldSession::key)
    }

    pub fn session(&mut self) -> Option<&mut (dyn WorldSession + 'static)> {
        self.session.as_deref_mut()
    }

    /// The options chosen for `game`.
    pub fn options_of(&self, game: Game) -> Vec<(OptionId, OptionValue)> {
        let chosen = self.options.iter().find(|(of, _)| *of == game);
        chosen.map_or_else(Vec::new, |(_, options)| options.clone())
    }

    /// Choose `options` for `game`: the next frame opens its world with them.
    pub fn choose(&mut self, game: Game, mut options: Vec<(OptionId, OptionValue)>) {
        options.sort();
        self.options.retain(|(of, _)| *of != game);
        self.options.push((game, options));
    }

    /// Hold `session` in place of the one held.
    pub fn hold(&mut self, session: Box<dyn WorldSession>) {
        self.session = Some(session);
        self.opened += 1;
    }

    pub fn drop_session(&mut self) {
        self.session = None;
    }
}

/// The world the World screen shows under `prefs`: the active game's where a title opens a world of it and its folder stands, else the first GTA VI build's with a folder set.
pub fn wanted(prefs: &Prefs, held: &Held) -> Option<WorldKey> {
    let active = prefs.active_game.and_then(|game| {
        let opens = game.is_gta_vi() || or_worlds::of_game(game).is_some();
        let install = prefs.game_paths.get(game)?;
        opens.then_some((game, install))
    });
    let (game, install) = active.or_else(|| super::gta_vi::install(prefs))?;
    install.is_dir().then(|| WorldKey {
        game,
        install: install.clone(),
        options: held.options_of(game),
    })
}

impl AppState {
    /// The session of the world the World screen shows, opened where the game, the install or the options changed; `None` with no install to show.
    pub fn world_session(&mut self) -> Option<&mut (dyn WorldSession + 'static)> {
        let wanted = wanted(&self.prefs, &self.world);
        if self.world.key() != wanted.as_ref() {
            // ⚠ The session held is let go first and its thread told to stop, but not waited for: one still taking its index ends when that is taken.
            self.world.drop_session();
            if let Some(key) = wanted {
                let session: Box<dyn WorldSession> = match key.game.is_gta_vi() {
                    true => Box::new(gta6::Gta6Session::new(key)),
                    false => Box::new(driven::DrivenSession::of_install(key, &self.prefs)),
                };
                self.world.hold(session);
            }
        }
        self.world.session()
    }
}
