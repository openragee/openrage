//! The maps engine: the map files a person streams into the game, as content of a project, with no UI.
//!
//! [`model`] is the map sets of a pack (the file `maps.json`), each naming the sources it is made of; [`layout`] says where each lands in a
//! resource, [`facts`] looks at the sources a module names and reads what the map files place and the type files declare, [`rules`] says
//! what is wrong as data with codes, severities and waivers, [`plan`] hands all of it to the plan of a build as a
//! [`or_project::Contribution`], [`verify`] reads a build back, [`adopt`] recognises a folder of maps and starts an empty one, and
//! [`import`] makes sources of game files. [`edit`] changes what an entity or an archetype says in its source, [`operation`] names each
//! of those changes as data with an inverse, [`scene`] says what a set places as boxes in the world, and [`gizmo`] is the manipulator
//! that moves, turns and scales one. `openrage-cli maps` runs the first part, and a list of operations, without the editor.
//!
//! The world model is `or-world`'s and the typed roots are `or_io::meta::typed`'s: this crate is the kind of content a project holds
//! and the edits made to it, and holds no reader and no schema of its own.
//!
//! ⚠ **Nothing here says a map loads in game.** A build says that every file it wrote reloads through the readers, which is all that
//! can be measured without running GTA V.

pub mod adopt;
pub mod edit;
pub mod facts;
pub mod gizmo;
pub mod import;
pub mod layout;
pub mod model;
pub mod operation;
pub mod plan;
pub mod rules;
pub mod sample;
pub mod scene;
pub mod verify;

pub use adopt::MapPack;
pub use facts::{
    inspect, inspect_named, Depth, Fact, Facts, Inside, Name, Names, Role, SourceFact,
};
pub use layout::{stream_path, STREAM, TYPE_FILE};
pub use model::{MapSet, Maps, Waiver, FILE};
pub use plan::{
    contribution, contributions, module_folder, module_folder_to_edit, planned, Planned,
};
pub use rules::{check, rule_of, Finding, Known, Place, Rule, Severity, RULES};
pub use scene::{scene, Placed, Scene};
