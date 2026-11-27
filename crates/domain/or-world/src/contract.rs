//! The contract a title's world sits behind: a title opens a world, an opened world answers with
//! its indexer, its map reader, its model reader and its look, and an edit store is opened the
//! same way. The shared code calls these and holds no test of which game it serves.
//!
//! ⚠ An id is a row of a linked [`Index`], and what it stands for is the title's: every reader is
//! handed the index its ids came from, and an id of another index names another map or none.

use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec3;
use or_job::{Cancel, Progress};

use crate::caps::WorldCaps;
use crate::collision::{BoxOverlap, GroundProbe, Hit, OrientedBox};
use crate::kept::{Inputs, Section, SectionError};
use crate::model::{
    AssetKey, Drawn, Draws, Feed, FrameFacts, Index, IndexFault, Instances, InteriorPlaced, Light,
    MapId, Model, Moment, Occluders, PlacementRow, Texture, TextureAsk,
};
use crate::policy::StreamPolicy;
use crate::sentence::Sentence;
use crate::slots::{Extras, OptionId, OptionKind, OptionValue, SwitchKind};
use crate::streaming::Node;

/// A game by the id its profile registers under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameId(pub &'static str);

/// Why a world did not open or did not answer.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    #[error("the title opens no world of the game `{0}`")]
    NotItsGame(&'static str),
    #[error("the option {0:?} is none of the title's, or holds a value it does not take")]
    Option(OptionId),
    #[error("{what} could not be read: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{what} does not read: {why}")]
    Unread { what: String, why: String },
    #[error("the rows of the index do not link: {0}")]
    Index(#[from] IndexFault),
    #[error("a section of the kept index does not read: {0}")]
    Section(#[from] SectionError),
    #[error("the map {0:?} is no row of the index")]
    NoSuchMap(MapId),
    #[error("the world states no {0}")]
    Lacks(&'static str),
    #[error("it was cancelled")]
    Cancelled,
}

/// A folder or an archive read above the install, which nothing writes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Layer {
    pub path: PathBuf,
}

/// What is opened: all of it keys the session and the kept file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldSpec {
    pub game: GameId,
    pub install: PathBuf,
    /// What the title lets a person choose at open, each by its id.
    pub options: Vec<(OptionId, OptionValue)>,
    /// Read only, above the install, in order.
    pub layers: Vec<Layer>,
}

impl WorldSpec {
    /// The install of `game` at `install`, with no option set and no layer.
    pub fn of(game: GameId, install: impl Into<PathBuf>) -> Self {
        Self {
            game,
            install: install.into(),
            options: Vec::new(),
            layers: Vec::new(),
        }
    }

    /// What the option `id` was set to.
    pub fn option(&self, id: OptionId) -> Option<&OptionValue> {
        (self.options.iter()).find_map(|(at, value)| (*at == id).then_some(value))
    }

    /// The world as a text of lines: two worlds that differ in anything but the order of their options differ in it.
    // ⚠ A path is written quoted and escaped: a line break in one starts no line of the text.
    pub fn identity(&self) -> String {
        let mut options = self.options.clone();
        options.sort();
        let mut out = format!("{}\n{:?}", self.game.0, self.install);
        for (id, value) in options {
            out.push_str(&format!("\n{}={value:?}", id.0));
        }
        for layer in &self.layers {
            out.push_str(&format!("\n+{:?}", layer.path));
        }
        out
    }
}

/// The keys a title may read an install with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys<'a> {
    /// None: a hand-built install opens with no key.
    None,
    /// A key snapshot.
    Snapshot(&'a Path),
    /// The user's own executable.
    Executable(&'a Path),
}

/// What an opening is given beside the world it opens.
#[derive(Clone, Copy)]
pub struct Opening<'a> {
    /// Where kept files and translated programs live.
    pub config: &'a Path,
    pub keys: Keys<'a>,
    pub cancel: &'a Cancel,
    pub progress: &'a Progress,
}

/// One value for each adapter crate.
pub trait WorldTitle: Sync {
    /// What names the title in a kept file's head and in the kept file's name.
    fn id(&self) -> &'static str;
    /// The games it opens a world of.
    fn games(&self) -> &'static [GameId];
    /// What is chosen at open, as data.
    fn options(&self) -> Vec<OptionKind>;
    fn open(
        &self,
        world: &WorldSpec,
        with: &Opening<'_>,
    ) -> Result<Arc<dyn WorldInstall>, WorldError>;
}

/// One world, opened; a title's is its own, and one made over values a caller lends is driven for as long as they are lent.
pub trait WorldInstall: Send + Sync {
    fn caps(&self) -> WorldCaps;
    fn policy(&self) -> StreamPolicy;
    fn extras(&self) -> Extras;
    /// The archives, and what else the index reads, each with what tells it changed.
    fn inputs(&self) -> Inputs;
    fn indexer(&self) -> &dyn WorldIndexer;
    fn maps(&self) -> &dyn MapReader;
    fn models(&self) -> &dyn ModelReader;
    fn look(&self) -> &dyn WorldLook;
    /// The ground under `at` as `probe` looks for it, `None` where there is none; an error where the world states no height of the ground.
    fn ground(
        &self,
        _index: &Index,
        _at: Vec3,
        _probe: &GroundProbe,
        _worker: &mut Worker,
    ) -> Result<Option<Hit>, WorldError> {
        Err(WorldError::Lacks("height of the ground"))
    }
    /// What a ray from `origin` along the unit vector `direction` first meets within `reach` metres; an error where the world states no collision query.
    fn ray(
        &self,
        _index: &Index,
        _origin: Vec3,
        _direction: Vec3,
        _reach: f32,
        _worker: &mut Worker,
    ) -> Result<Option<Hit>, WorldError> {
        Err(WorldError::Lacks("collision query"))
    }
    /// The pieces of each bound the box `region` holds or crosses, each bound by the title's name of it; an error where the world states no collision query.
    fn overlap(
        &self,
        _index: &Index,
        _region: &OrientedBox,
        _worker: &mut Worker,
    ) -> Result<Vec<(u64, BoxOverlap)>, WorldError> {
        Err(WorldError::Lacks("collision query"))
    }
}

/// What reads an install's rows and links them.
pub trait WorldIndexer: Sync {
    /// The version of what a build reads of each file: a kept file of another is read again whole.
    fn rules(&self) -> u32;
    /// The rows of the archive at `archive` among the inputs, with names unresolved.
    fn rows(&self, archive: usize, out: &mut Section) -> Result<(), WorldError>;
    /// What is derived across the archives' rows.
    fn whole(&self, archives: &[Section], out: &mut Section) -> Result<(), WorldError>;
    /// The linked index, names turned to ids: called after every load, from nothing but the sections.
    fn link(&self, archives: &[Section], whole: &Section) -> Result<Index, WorldError>;
    /// What the map `map` places; an error where the world states no placement rows.
    fn placements(
        &self,
        index: &Index,
        map: MapId,
        out: &mut Vec<PlacementRow>,
    ) -> Result<(), WorldError>;
}

/// What one reading thread keeps between reads, such as its open files.
#[derive(Default)]
pub struct Worker {
    state: Option<Box<dyn Any + Send>>,
}

impl Worker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The state of the kind `T` this worker keeps, made where it holds none or another kind.
    pub fn state<T: Default + Send + 'static>(&mut self) -> &mut T {
        if !self.state.as_ref().is_some_and(|held| held.is::<T>()) {
            self.state = Some(Box::new(T::default()));
        }
        match self.state.as_mut().and_then(|held| held.downcast_mut()) {
            Some(state) => state,
            // The line above put a `T` there.
            None => unreachable!("a worker's state is of the kind just made"),
        }
    }
}

/// What reads a map's body.
pub trait MapReader: Sync {
    fn read(
        &self,
        index: &Index,
        map: MapId,
        worker: &mut Worker,
    ) -> Result<Arc<dyn MapBody>, WorldError>;
    /// The body of the interior `placed` of `map`, its nodes placed in the world by its instance in `body`.
    // ⚠ `body` is the map's as it stands, an edit session's where an apply changed it: where the
    // instance stands is read from it and never from the map as the install holds it.
    fn interior(
        &self,
        index: &Index,
        map: MapId,
        body: &dyn MapBody,
        placed: &InteriorPlaced,
        worker: &mut Worker,
    ) -> Result<Arc<dyn MapBody>, WorldError>;
}

/// A map body, the title's own type, read once; a title finds its own type in one it is handed back through `Any`.
pub trait MapBody: Any + Send + Sync {
    /// Its nodes, a parent named by [`MapId::key`] or by [`crate::model::interior_key`].
    fn nodes(&self) -> &[Node];
    /// What draws each of its nodes at `at`, pushed in the nodes' order: asked once a body where the moment or the body changed, never once a node.
    fn draws(&self, at: &Moment, out: &mut Vec<Draws>);
    fn lights(&self) -> &[Light];
    fn occluders(&self) -> &Occluders;
    fn interiors(&self) -> &[InteriorPlaced];
    /// The instances of what `drawn` draws of it: called once a loaded body a selection.
    fn lay(&self, drawn: &Drawn<'_>, at: &Moment, out: &mut Instances);
    /// The bytes it holds, as the budget counts them.
    fn bytes(&self) -> u64;
}

/// What reads models and textures, in batches the title groups as its files hold them.
pub trait ModelReader: Sync {
    /// Each model of `asks` that reads, with its ask.
    fn models(
        &self,
        index: &Index,
        asks: &[AssetKey],
        worker: &mut Worker,
        out: &mut Vec<(AssetKey, Model)>,
    );
    /// Each texture of `asks` that reads, with its ask.
    fn textures(
        &self,
        index: &Index,
        asks: &[TextureAsk],
        worker: &mut Worker,
        out: &mut Vec<(TextureAsk, Texture)>,
    );
}

/// What a title's day is to the shared frame.
pub trait WorldLook: Sync {
    /// What is switched while the world of `index` is open: a title whose switches its files state reads them from its link.
    fn switches(&self, index: &Index) -> Vec<SwitchKind>;
    /// What the moment alone decides, the rest of the title's day handed to `out` by name.
    fn frame(&self, at: &Moment, out: &mut dyn Feed) -> FrameFacts;
}

/// An operation by the title's number of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(pub u32);

/// One operation a title's store applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationKind {
    pub id: OperationId,
    pub label: Sentence,
}

/// A file an edit changes, by the title's own name of it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address(pub String);

/// What says a file stands as an operation took it to stand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Guard {
    /// A token of a file no edit of the session has written.
    Unwritten(u64),
    /// A digest of what the session holds of it.
    Written(Box<[u8]>),
}

/// One operation as any title's store takes it: its kind, the files it changes each with its guard, and the title's own encoding of the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub kind: OperationId,
    pub at: Vec<(Address, Guard)>,
    pub body: Box<[u8]>,
}

/// What an apply came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// What takes the store back to where it stood.
    pub inverse: Envelope,
    /// The maps whose body changed.
    pub maps: Vec<MapId>,
}

/// One file a flush wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub at: Address,
    pub bytes: u64,
}

/// What a file stood as before any edit, to compare with, put back to and merge against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base(pub Arc<[u8]>);

/// Why an edit store did not open or did not apply.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("`{}` changed since the operation was made", .0 .0)]
    Stale(Address),
    #[error("the store applies no operation {0:?}")]
    NoSuchOperation(OperationId),
    #[error("the operation does not read: {0}")]
    Malformed(String),
    #[error("the operation names `{}`, which the store does not hold", .0 .0)]
    NoSuchAddress(Address),
    #[error("{what} could not be written: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    World(#[from] WorldError),
}

/// Where an edit store is opened: over an opened world, lent for `'w`, and a folder where the store is one.
#[derive(Clone, Copy)]
pub struct EditPlace<'a, 'w> {
    pub install: &'a Arc<dyn WorldInstall + 'w>,
    /// The index the session's map ids are rows of.
    pub index: &'a Arc<Index>,
    pub folder: Option<&'a Path>,
}

/// A title's edit store, opened as a world is.
pub trait EditTitle: Sync {
    fn operations(&self) -> Vec<OperationKind>;
    /// A session over the world of `place`, which it may hold for as long as that world is lent.
    fn open<'w>(&self, place: &EditPlace<'_, 'w>) -> Result<Box<dyn EditSession + 'w>, EditError>;
}

/// An open edit store: what was applied is held, and written at a flush.
pub trait EditSession: Send {
    /// What says `at` stands as it does now.
    fn guard(&self, at: &Address) -> Guard;
    /// Apply `op`, refused where one of its guards is stale.
    fn apply(&mut self, op: &Envelope) -> Result<Applied, EditError>;
    /// The body of `map` where an apply changed it: asked before the reader, and the same value until the next apply changes it.
    fn body(&self, map: MapId) -> Option<Arc<dyn MapBody>>;
    fn flush(&mut self) -> Result<Vec<Written>, EditError>;
    /// What `at` stood as before any edit; `None` where the store has no comparison, put-back or merge.
    fn base(&self, at: &Address) -> Option<Base>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_keeps_its_state_between_reads_and_makes_another_for_another_kind() {
        let mut worker = Worker::new();
        worker.state::<Vec<u32>>().push(7);
        worker.state::<Vec<u32>>().push(9);
        assert_eq!(*worker.state::<Vec<u32>>(), [7, 9]);
        assert_eq!(*worker.state::<String>(), "");
        assert!(
            worker.state::<Vec<u32>>().is_empty(),
            "a worker holds one kind"
        );
    }

    #[test]
    fn a_world_answers_the_option_it_was_set_and_no_other() {
        let mut world = WorldSpec::of(GameId("a-game"), "an-install");
        assert_eq!(world.option(OptionId(2)), None);
        world.options = vec![
            (OptionId(1), OptionValue::On(true)),
            (OptionId(2), OptionValue::Choice(5)),
        ];
        assert_eq!(world.option(OptionId(2)), Some(&OptionValue::Choice(5)));
        assert_eq!(world.option(OptionId(3)), None);
    }
}
