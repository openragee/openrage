//! The maps engine: the map files a person streams into the game, as content of a project, with no UI.
//!
//! [`model`] is the map sets of a pack (the file `maps.json`), each naming the sources it is made of; [`layout`] says where each lands in a
//! resource, [`facts`] looks at the sources a module names and reads what the map files place and the type files declare, [`rules`] says
//! what is wrong as data with codes, severities and waivers, [`plan`] hands all of it to the plan of a build as a
//! [`or_project::Contribution`], [`manifest`] is the manifest of its maps, written from the manifests of the game the module keeps,
//! [`verify`] reads a build back, [`adopt`] recognises a folder of maps and starts an empty one,
//! [`import`] makes sources of game files, and [`mapping`] reports and adopts a mapping that comes in as a folder or a zip. [`edit`]
//! changes what an entity or an archetype says in its source, [`operation`] names each
//! of those changes as data with an inverse and holds the sources of a pack open as a session that writes them once, [`delete`] plans
//! the removal of an entity with its chain of far models across the map files of a pack as a list of them, [`first_edit`] is the edit
//! of an object the game placed, which imports its map as a source and is taken back whole, [`scene`] says what a set
//! places as boxes in the world, and [`gizmo`] is the manipulator
//! that moves, turns and scales one. [`place`] reads what a person pastes into a search field as a position, a hash or text, and
//! [`script_text`] writes a position as the text a script takes. [`doors`] finds the doors of a set, pairs them, opens each against
//! what stands near it and holds them to the door rules, and [`door_text`] writes a selection of them as the text a door script
//! takes. `openrage-cli maps` runs the first part, a list of operations and the doors, without the editor.
//!
//! The world model is `or-world`'s and the typed roots are `or_io::meta::typed`'s: this crate is the kind of content a project holds
//! and the edits made to it, and holds no reader and no schema of its own.
//!
//! [`changes`] says what a map or a type file changed of the base its source keeps, by object and not by field: the entities and the
//! archetypes added, deleted and changed, and the ones no guid and no place tells from a twin. `openrage-cli maps changes` prints it.
//! [`merge`] merges two copies of one map, or of one type file, against the file both were made from, with what `changes` says each
//! changed: a conflict where they disagree, and never a guess. `openrage-cli maps merge` runs it. [`put_back`] plans what makes one
//! entity state what the base states again, a deleted one with the chain of far models its deletion took, as a list of operations.
//! `openrage-cli maps put-back` runs it.
//!
//! [`roads`] is the path regions a folder holds as sources: the operations that edit their nodes and links, each with its inverse, and
//! the rules that say what is wrong in them. `openrage-cli maps roads` lists them and applies a list of operations.
//!
//! ⚠ **Nothing here says a map loads in game.** A build says that every file it wrote reloads through the readers, which is all that
//! can be measured without running GTA V.

pub mod adopt;
pub mod changes;
pub mod collision;
pub mod delete;
pub mod door_text;
pub mod doors;
pub mod edit;
pub mod facts;
pub mod first_edit;
pub mod gizmo;
pub mod import;
pub mod layout;
pub mod manifest;
pub mod mapping;
pub mod merge;
pub mod model;
pub mod operation;
pub mod place;
pub mod plan;
pub mod put_back;
pub mod roads;
pub mod rules;
pub mod sample;
pub mod scene;
pub mod script_text;
pub mod verify;

pub use adopt::MapPack;
pub use facts::{
    inspect, inspect_named, Depth, Fact, Facts, Inside, Name, Names, Role, SourceFact,
};
pub use layout::{stream_path, MAP_LINE, STREAM, TYPE_FILE};
pub use model::{MapSet, Maps, Waiver, FILE};
pub use plan::{
    contribution, contribution_for, contributions, module_folder, module_folder_to_edit, planned,
    planned_in, Planned, Shape,
};
pub use rules::{check, rule_of, Finding, Known, Place, Rule, Severity, RULES};
pub use scene::{scene, Placed, Scene};
