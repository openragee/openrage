//! The project container: what a `.openrage-proj` holds and declares, the plan of a build as data, and the executor that writes it.
//!
//! A project has no type. It holds **modules** (Content on screen) and declares **outputs**
//! (Builds on screen); [`container`] reads and writes those two keys, [`contribution`] is what a
//! module hands an output, [`plan`] merges those into what an output will write, [`diff`] says what
//! it changed since the last build, and [`build`] writes it. The editor and `openrage-cli project`
//! share all of it.
//!
//! ⚠ **A build never says a pack loads in game.** It says that every file it wrote equals what
//! the plan named and reloads through the readers that exist for it, which is all that can be
//! measured without running GTA V. [`acceptance`] is where a person writes down a run of the game.

pub mod acceptance;
pub mod adopt;
pub mod build;
pub mod built;
pub mod check;
pub mod container;
pub mod content;
pub mod contribution;
pub mod diff;
mod dlc;
pub mod extension;
pub mod gta6;
mod left_out;
pub mod links;
pub mod lock;
mod merge;
mod oiv;
pub mod paths;
pub mod place;
pub mod plan;
pub mod project;
mod readers;
pub mod reference;
pub mod resource;
pub mod server;
pub mod sizes;
pub mod snapshot;
pub mod split;
pub mod template;
pub mod watch;

pub use build::{
    build, build_all, build_all_observed, build_all_watched, build_all_with, build_observed,
    build_watched, build_with, built_resources, check_reproducible, clean, verify, BuildError,
    BuildStep, Built, Cleaned, Cook, Cooked, Loss, Memory, MetadataCount, Observe, Reloaded,
    Reproducible, Unchanged, Verification,
};
pub use built::{built_files, BuiltFiles, Folder};
pub use container::{Container, Module, ModuleKind, Output, OutputKind, Target};
pub use contribution::{
    ArchiveContents, Claim, Contribution, Cooks, Figure, Item, ManifestEntry, SourceFolder,
};
pub use diff::{diff, since_last_build, Diff, Moved};
pub use dlc::{LEVEL_FOLDER, MAP_DATA};
pub use paths::{
    build_root, find_sources, project_folder, relative_inside, resolve_member, shadow_root,
    store_path, BUILD_DIR, PROJECT_EXTENSION, SEARCH_DEPTH, SHADOW_DIR,
};
pub use plan::{
    cannot_ship, files_contributions, files_contributions_with, files_without_a_type, plan,
    plan_with, Budget, Limits, NameCheck, NoSuchOutput, Origin, Plan, PlannedFile, Presence,
    Registration, Source, Totals,
};
pub use project::{replace_file, Member, Project, SCHEMA};
pub use split::{plans, plans_with};
pub use watch::{Cancel, Progress, Watch};
