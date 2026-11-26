# Unified world plan

How the world viewer and the world editor become one title-neutral core with one adapter crate for each game, what the tree holds today, the contract between the two, how a world is drawn, and the order and the briefs of the rows that build it.

Status: the plan of 2026-11-26, written on the owner's decisions of that day ([the shape](Project_Context.md#the-world-viewer-and-editor-become-one-core-and-an-adapter-crate-for-each-game-2026-11-26), [who lifts the GTA VI viewer](Project_Context.md#this-programme-lifts-the-gta-vi-viewer-out-of-the-app-and-the-other-gta-vi-sessions-pause-2026-11-26), [the drawing](Project_Context.md#each-game-draws-its-world-with-its-own-shader-programs-in-one-shared-frame-host-2026-11-26)); [R71](Roadmap.md#r71-one-world-for-every-title) says where each row stands.

## Verdict

The owner asked for a world viewer and editor that "respect DRY and support multiple games", with the duplicated code removed and each game "allowed to make their own little edits". The agreed shape holds: **one core, `or-world`, that names no game, and one adapter crate for each of GTA V, GTA VI and Red Dead Redemption 2**, with one world session in the app, one frame host in the renderer, and each game's own shader programs run inside that host.

The code agrees with the shape and disagrees with five of the proposal's steps, and this plan follows the code:

1. **Lifting the GTA VI viewer is not a pure move.** At `9fdf0fee`, 19 of its 39 source files name the renderer, the graphics library or the app, and a domain crate may name none of them. What moves unchanged is the other 20 files; the rest is moved with its output types rewritten ([R71b](#the-gta-vi-world-behind-the-contract-r71b)).
2. **The GTA VI world does not run GTA VI's programs today.** Its surfaces are drawn by OpenRage's own shader and lit by GTA V Legacy's lighting passes. That picture is kept as GTA VI's fallback while the programme moves it, and running its own programs is a row of its own ([R71h](#the-programs-of-gta-vi-over-a-world-r71h)).
3. **Nothing draws the GTA V world, and the game's passes exist for one drawable.** Los Santos is new work in three rows: drawn in the fallback, then through Legacy's programs, then through Enhanced's, which no bake translates ([R71e](#the-gta-v-world-drawn-in-the-fallback-pass-r71e), [R71f](#the-render-profile-and-the-programs-of-gta-v-legacy-r71f), [R71g](#the-programs-of-gta-v-enhanced-r71g)).
4. **The editor is not reached through the title's trait.** GTA V's edit store is bound to the project and sits above its adapter, so the registry pairs a title with its store ([the contract](#the-contract)).
5. **A render profile is data in the adapter, and every line that touches the device stays in the renderer.** A translator is handed to the host by the app, as it is today ([drawing](#drawing)).

The order of work therefore has more rows than the proposal had steps ([the order](#the-order-of-work)).

## What it rests on

- **The tree at `9fdf0fee`**, read on 2026-11-26 by three readers and spot-checked for this plan: every line count here is `wc -l` at that revision, and every test count is the `#[test]` attributes a `grep` finds there. Nothing was built or run for the readings.
- **The owner's three decisions of 2026-11-26**, recorded in [Project_Context.md](Project_Context.md) under the links of the status line.
- **[World_Editor_Plan.md](World_Editor_Plan.md)**, the plan of [R70](Roadmap.md#r70-the-gta-v-world-editor), whose seam between the titles, whose table of where the code goes and whose ruling 9 this plan replaces ([what it changes for R70](#what-it-changes-for-r70)).
- **[Shader_Pipeline_Design.md](Shader_Pipeline_Design.md)** for what a translated GTA V program is and what was measured of it, and [GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md) and [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) for GTA VI.
- **What is not known is said so.** A figure nobody measured is named as unmeasured, with the measurement that would settle it, in the row that needs it.

## What the tree holds today

### GTA VI

The app holds two GTA VI worlds, each a process-wide singleton with its own index and its own kept file. The World screen streams and draws an install; the Map screen lays a plan of an archive over its scene and edits a scanned map body as a document. Lines and tests at `9fdf0fee`:

| Part | Where | Files | Lines | Tests |
|---|---|---|---|---|
| The World screen's viewer | `src/state/gta6_worldview/` | 55 | 18,010 | 189 |
| The bridge that draws it, which is the host today | the world files of `src/render/viewport/` | 13 | 4,651 | 49 |
| Its panels | the world viewer files of `src/ui/panels/` | 5 | 2,211 | 13 |
| The Map screen's world store | `src/state/gta6_world.rs`, `src/state/gta6_world_fill.rs` | 3 | 1,765 | 9 |
| Map edits as steps of a document's undo | `src/state/gta6_map.rs` and its neighbours | 8 | 4,399 | 23 |
| Navigation and collision edits | the navmesh files of `src/state/` and `src/state/gta6_collision_edit.rs` | 7 | 2,941 | 26 |

What matters of it for this plan, each at `9fdf0fee`:

- **The viewer is one static.** `viewer()` in `src/state/gta6_worldview/mod.rs` locks a `LazyLock<Mutex<Viewer>>`, and 23 call sites in nine files reach it. The folder holds 15 statics and the Map-screen and edit files 17 more. Three are single-valued for the process and wrong for two sessions: the exposure of `src/state/gta6_worldview/sky.rs`, the night maps of `src/state/gta6_worldview/sky_night.rs`, which are those of whichever install was taken last, and the one-slot sky radiance worker.
- **No test reads an install.** The 189 tests of the viewer build index rows and map bodies by hand; none runs the index build, the archive scan or a reader against an archive, so the read path is proven only by shots runs on the real build.
- **The selection carries no identity.** An instance the streamer selects is a model, its rows, a sphere, a fade and a tint: no node, map or entity. Nothing picks, selects or moves in the World screen.
- **Every GTA VI edit is a step of a document.** Moving, adding, removing or retyping an entity edits the value tree or the sections of a scanned body opened as a document, undone one document at a time and saved as a loose resource. It has no named operation, no staleness guard, no base to compare with and no project.
- **The map readers are not declarations.** They read a body by member hash and offset checked against the body's own class table, and three such readers live in the app: `src/state/gta6_worldview/index_types.rs`, `src/state/gta6_worldview/lights_lod.rs` and `src/state/gta6_worldview/lights_index.rs`.
- **The look is in two layers below and above the adapter's.** The day, sun, weather, wind, exposure, sky and cloud tables are read in `crates/title/or-title/src/` (the `gta6_day` files and their neighbours), and the entity edits and the link renumbering are in `crates/io/or-io/src/gta6/map_grow.rs`.

### GTA V

GTA V's world is headless: `openrage-cli world` reaches it and no screen draws it. Lines at `9fdf0fee`:

| Part | Where | Lines |
|---|---|---|
| The world model: maps, types, extensions, groups, manifests, state, creation | `crates/domain/or-world/src/`, all but streaming, collision geometry and the transform | 12,996 |
| The index of an install, kept on disk, and its mount source | `crates/domain/or-depends/src/world_index.rs` and its folder | 1,910 |
| The streaming driver and its map reader | `crates/domain/or-depends/src/world_forest.rs` and its folder | 816 |
| Collision near a position, find, placements | `crates/domain/or-depends/src/world_collision.rs`, `crates/domain/or-depends/src/find.rs` and its folder, `crates/domain/or-depends/src/placement.rs` | 1,451 |
| The edit vocabulary, its session, the first edit, delete, compare, merge, rules | `crates/domain/or-maps/src/` | 22,178 |
| The command line's world verbs | the world files of `crates/tools/or-cli/src/` | 6,953 |

- **`or-world` is GTA V's model, not a neutral core.** Its header states two invariants a core must respect: nothing there copies the tree a reader produced, and nothing keys on a field's spelling. The header of `crates/domain/or-world/src/map_data.rs` records what a second model that forgot fields cost another crate, so the core's scene is a read model and an edit stays on each title's own records.
- **The edit vocabulary is bound to the project.** An `Operation` of `crates/domain/or-maps/src/operation.rs` addresses a source folder of a pack and an index into the file it cooks to, carries a record of GTA V's entity class, and is guarded by the digest of the source's XML. `or-maps` depends on `or-project` and `or-source`, which the core cannot.
- **GTA V deletes with renumbering twice.** `crates/domain/or-world/src/execute.rs` does it over value trees, reached from `openrage-cli world` and written only where a schema snapshot is staged; `crates/domain/or-maps/src/delete.rs` does it over typed roots.
- **The mount is GTA V's own.** `build_mounted_game_with` in `or-title` is hard-wired to `common.rpf`, the platform archives, the update archive and the DLC packs, and `WorldSource` in `crates/domain/or-depends/src/world_index.rs` speaks logical paths and RPF7 entry facts.

### Red Dead Redemption 2

An install is browsed as a folder of archives and nothing of its world is read. RPF8 archives open, any RSC8 resource opens as its virtual and physical bytes, and one format is declared, the texture dictionary of `crates/io/or-io/src/rdr2/texture.rs`, read only ([R56](Roadmap.md#r56-rdr2-the-second-container)). There is no mount: the function that mounts GTA V wants `common.rpf` and RDR2 ships another name. A tagged archive lists its files by hash, since its names blob is not under the cipher the snapshot holds. Its map, type, model and bound files are listed by the archive's extension table and declared by nothing, and whether its maps and types are resources or metadata is not checked. Its shader containers are not measured.

### The renderer

Three sets of game programs run today in three ways, and the GTA VI world runs none of GTA VI's:

| What is drawn | Its surfaces | Its lighting and post | Where the programs come from |
|---|---|---|---|
| A GTA V drawable in Shaded, and particles | GTA V Legacy's own programs | GTA V Legacy's own | a pack the helper `or-shader-tool` bakes into the configuration folder |
| A GTA VI model in Shaded | GTA VI's own mesh and pixel programs, lowered in the process | OpenRage's own shaders | the install's effect containers, translated on a worker |
| The GTA VI world | OpenRage's own `WORLD_SHADER` of `crates/runtime/or-renderer/src/world/shader.rs`, in families the app maps each material onto | GTA V Legacy's lighting passes from that pack, and OpenRage's own shaders for GTA VI's post, sky and water | no GTA VI program |
| GTA V Enhanced | not run | not run | its programs are read for the definition database only |
| RDR2 | nothing | nothing | not opened |

- **The host is in the app and typed on GTA VI.** Pages, the texture pool, culling, occlusion, budgets and readiness are the world files of `src/render/viewport/`; they name GTA VI through the viewer's model, material, texture, instance and chunk types and the occluder types of `or-io`. `src/render/viewport/world_budget.rs` and `src/render/viewport/world_pages.rs` import no title type.
- **The frame is one function.** `render_deferred` in `crates/runtime/or-renderer/src/viewport/game_frame.rs` draws a GTA V drawable and the GTA VI world, branching on whether the day carries GTA VI's post. The bloom and the ambient occlusion pick passes of their effects by number, and a pass in the pack's manifest carries no render state.
- **A title-neutral draw list exists.** The renderer's `set_world` takes instances and draws with no title type, and the overlay pass after the frame loads the lit colour and the depth, so lines and plain meshes already draw over any world.
- **`or-renderer` names both games.** Of the 29,535 lines under `crates/runtime/or-renderer/src/game_shaders/` at `9fdf0fee`, the readers count 2,971 non-test lines as title-neutral engine, 9,232 as GTA V's and 5,478 as GTA VI's, by reading each module.
- **The translators are tools.** `or-shader-front` holds the three front ends and `or-cli` the GTA VI glue; `or-renderer` depends on neither, and the app reaches them through `or-cli` and through the trait of `src/state/gta6_shader_translator.rs`, which a test stands in for.

### What is written twice

Each row is a duplicate the programme ends, at `9fdf0fee`:

| What | GTA V | GTA VI | Shared today |
|---|---|---|---|
| The index of an install | `crates/domain/or-depends/src/world_index.rs` | `src/state/gta6_worldview/index.rs` and five neighbours | nothing |
| A kept file of rows, checked against the archives' size and time | `crates/domain/or-depends/src/world_index/kept.rs` | `src/state/gta6_worldview/index_keep.rs`, and a second in `src/state/gta6_world_fill.rs` | the hash that names the file |
| The driver around the forest: wanted maps, read, insert, select, instances | `crates/domain/or-depends/src/world_forest.rs` | `src/state/gta6_worldview/stream.rs` and its neighbours | `Forest` of `crates/domain/or-world/src/streaming.rs` |
| What is switched on at an hour | `crates/domain/or-world/src/state.rs` | `src/state/gta6_worldview/index_states.rs` | nothing |
| A map's extents grown to hold an entity | `crates/domain/or-maps/src/edit.rs`, `crates/domain/or-world/src/extents.rs` | `src/state/gta6_map_extents.rs` | the arithmetic of `crates/domain/or-world/src/transform.rs` |
| Removing an entity and repairing links across maps | `crates/domain/or-world/src/execute.rs`, `crates/domain/or-maps/src/delete.rs` | `crates/io/or-io/src/gta6/map_grow.rs`, `src/state/gta6_map_entity.rs` | nothing |
| The undo of the Map screen | `src/state/workbench/maps.rs` | the document's own steps | the screen |
| The box scene of the Map screen | `crates/domain/or-maps/src/scene.rs` | `src/state/gta6_map.rs` | the scene type and the manipulator |

Inside GTA VI alone, the same map body is streamed three times around one camera by three loops with their own pools (the forest's, the occluders' and the lights'), the block that opens an archive once for a worker and reads a body is written out in six files of the viewer, and the bucket and family enums are stated once in the app and once in the renderer with a test that every variant has its namesake.

## The shape

### The crates

| Crate | Layer | Owns | Depends on |
|---|---|---|---|
| `or-world`, the core | domain | The neutral read model (index rows, forest nodes, a model and a texture as CPU data, the selection), the contract, the stream policy, the driver around the forest and its world thread and reader pool, the one kept-file mechanism, the queries, collision geometry, the transform, the manipulator and the box scene, the edit envelope with its session and undo, the rules slot, the conformance kit and the fake title | `or-job`, `or-title` for the game's id alone |
| or-world-gta5 | domain | GTA V's map and type model, its index rows and their codec, its mount source, its policy, its bound-to-shape conversion, its find and placements, its render profile as data. One title answering for the Legacy and the Enhanced game | the core, `or-io`, `or-title` |
| or-world-gta6 | domain | GTA VI's index, scan and mount binding, its map, interior, light and occluder readers, its materials, its build's streaming rules, its look, its edit store over documents, its render profile as data | the core, `or-io`, `or-title` |
| or-world-rdr2 | domain | What [R71k](#red-dead-redemption-2-r71k) measures it needs | the core, `or-io`, `or-title` |
| or-worlds, the registry | domain | One function that lists each title with its edit store, in the manner of `kinds` in `crates/domain/or-content/src/lib.rs` | every adapter, `or-maps` |
| `or-maps` | domain | GTA V's edit store and its project content kind | the core, or-world-gta5, `or-project`, `or-source` |
| `or-depends` | domain | GTA V's dependency graph | or-world-gta5 |
| `or-renderer`, the host | runtime | The frame host and the fallback pass, typed on the core's model | the core, as it depends on `or-particle` and `or-interchange` today |

Dependencies still run foundation, io, title, domain, runtime, app, and the new edges all point down or sideways within a layer. `or-renderer` gains an edge to `or-world`, which [World_Editor_Plan.md](World_Editor_Plan.md#where-the-code-goes) said it lacks; it points down the chain. `or-depends` and `or-maps` depend on the GTA V adapter where they depended on `or-world`. The registry crate depends on every adapter, as `or-content` depends on every kind of content, so the app and `openrage-cli` list the titles once.

No adapter depends on another, on `or-renderer`, on a tool crate or on the app. File reading stays in `or-io` for each game: GTA V's as declarations (D15), GTA VI's as the readers by member hash they are, and the three such readers the app holds go down to `or-io` with their tests in R71b. Unification happens above the bytes.

### What moves from where

| From | To | In |
|---|---|---|
| The policy constants and the `with_exact_children` flag of `crates/domain/or-world/src/streaming.rs`; `streams_in` of `crates/domain/or-depends/src/world_forest.rs` | the core's stream policy, as fields with defaults | R71a |
| `Pose` of `src/ui/panels/world_viewer.rs` | the core, as the camera a session streams by | R71a |
| The 20 viewer files that name neither the renderer nor the app (5,435 lines at `9fdf0fee`) | or-world-gta6, unchanged but for imports; the world thread and the reader pool to the core | R71b |
| The rest of `src/state/gta6_worldview/`; `crates/domain/or-world/src/streaming_rules.rs` | or-world-gta6, with the output types rewritten | R71b |
| The mechanics of the world files of `src/render/viewport/` | `or-renderer`, typed on the core's model | R71c |
| The GTA V modules of `crates/domain/or-world/src/`; the world half of `crates/domain/or-depends/src/` with the catalogue and tables its index reads | or-world-gta5 | R71d |
| `crates/domain/or-maps/src/place.rs`, `crates/domain/or-maps/src/gizmo.rs`, `crates/domain/or-maps/src/scene.rs` and `Pose` of `crates/domain/or-maps/src/edit.rs` | the core | R71d |
| The three GTA VI entity edit files of `src/state/` | or-world-gta6, as its edit store | R71i |
| `src/state/gta6_world.rs`, `src/state/gta6_world_fill.rs` | the placement rows of or-world-gta6 | R71j |

## The contract

The outline below is what [R71a](#the-contract-the-kit-and-the-fake-title-r71a) builds. Its names are proposals; a field is fixed against both titles as it is written, and the contract is frozen when both draw a world, at the end of R71e. Until then a change to it is an ordinary change, and after it a change must leave every adapter's kit passing.

```rust
/// One value for each game: the whole of what an adapter crate exports.
pub trait WorldTitle: Sync {
    fn games(&self) -> &'static [Game];            // or_title's id; GTA V answers for two
    fn caps(&self) -> WorldCaps;                   // what it has and what it writes back, as data
    fn policy(&self) -> StreamPolicy;              // the core's defaults with this game's overrides
    fn extras(&self) -> Extras;                    // its layers, rules, operations and panel sections
    fn profile(&self) -> Option<&dyn RenderProfile>; // none: drawn in the fallback pass
    fn open(&self, root: &Path, with: &Opening) -> Result<Arc<dyn WorldInstall>, WorldError>;
}

/// What the caller that has a place for files hands in; an adapter asks the app for nothing.
pub struct Opening<'a> {
    pub config: &'a Path,           // where kept files and translated programs live
    pub keys: Option<&'a Path>,     // a key snapshot, where the title needs one
    pub cancel: &'a Cancel,
    pub progress: &'a Progress,
}

/// One install, opened. The adapter's own mount stays behind it.
pub trait WorldInstall: Send + Sync {
    fn archives(&self) -> Vec<ArchiveStamp>;       // path, size, time: what a kept file is checked against
    fn indexer(&self) -> &dyn WorldIndexer;        // archives to neutral index rows
    fn maps(&self) -> &dyn MapReader;              // one map body to forest nodes
    fn models(&self) -> &dyn ModelReader;          // bytes to a model and a texture as CPU data
    fn look(&self) -> &dyn WorldLook;              // switches, the day, lights, water
}

pub trait WorldIndexer: Sync {
    fn rows(&self, archive: usize, out: &mut ArchiveRows, with: &Opening) -> Result<(), WorldError>;
    fn placements(&self, map: MapId, out: &mut Vec<PlacementRow>) -> Result<(), WorldError>; // by capability
}
pub trait MapReader: Sync {
    fn read(&self, map: MapId, worker: &mut Worker) -> Result<MapBody, WorldError>;
}
pub trait ModelReader: Sync {
    fn model(&self, model: ModelId, worker: &mut Worker) -> Option<Model>;
    fn texture(&self, texture: TextureId, pixels: u32, worker: &mut Worker) -> Option<Texture>;
}
pub trait WorldLook: Sync {
    fn switches(&self) -> Switches;                // named toggles and groups, each with a default at an hour
    fn frame(&self, at: &Moment, out: &mut dyn Feed) -> FrameFacts;
}

/// The store an edit is applied to. Paired with a title by the registry, not by the title.
pub trait WorldEdit: Sync {
    fn operations(&self) -> &[OperationKind];
    fn guard(&self, at: &Address) -> Guard;        // what an operation states it applies to
    fn apply(&self, op: &Envelope) -> Result<Applied, EditError>; // returns the inverse
    fn base(&self, at: &Address) -> Option<Base>;  // none: no comparison, no put-back, no merge
}

// The neutral read model. Rows are the adapter's to encode; ids are row numbers of its index.
pub struct MapId(pub u32);  pub struct ModelId(pub u32);  pub struct TextureId(pub u64);
pub struct Name { pub hash: u64, pub text: Option<Arc<str>> }   // a 32-bit name widens
pub struct MapRow { pub name: Name, pub boxes: MapBox, pub parents: Vec<MapId>, pub held: Option<Held>, pub group: Option<GroupId> }
pub struct ArchetypeRow { pub name: Name, pub sphere: [f32; 4], pub lod_dist: f32, pub model: Option<ModelId> }
pub struct MapBody { pub nodes: Vec<Node>, pub lists: Vec<List>, pub interiors: Vec<InteriorPlaced> }
pub struct Model { pub lods: Vec<Lod> }           // a mesh: vertex streams with their layout, indices,
                                                  // a material record for the profile, a surface for the fallback
pub struct Texture { pub width: u32, pub height: u32, pub format: BlockFormat, pub levels: Vec<Vec<u8>> }
pub struct Instance { pub node: NodeId, pub model: ModelId, pub rows: [[f32; 4]; 3], pub fade: f32, pub tint: u32 }
```

Each part stands on code that exists and would sit behind it:

| Part | What sits behind it today |
|---|---|
| The title and the opened install | `GameProfile` and its registry in `crates/title/or-title/src/game/` name a game; nothing names a mount, which is three different things. The install is opened by the adapter because of that: a `Vfs` of logical paths for GTA V, an archive and a byte offset found by a table or a scan for GTA VI, nothing yet for RDR2 |
| The opening | The GTA VI index asks the app for its configuration folder and its key snapshot from inside the module (`src/state/gta6_worldview/index_mount.rs`); GTA V's is handed a mount |
| The indexer and the kept file | `WorldSource` and the rows of `crates/domain/or-depends/src/world_index.rs`; `MapRow`, `ArchetypeRow` and `ModelRef` of `src/state/gta6_worldview/index.rs`. A row is filled an archive at a time, so that GTA V keeps the refresh that reads only what changed |
| The map reader | `crates/domain/or-depends/src/world_forest/read.rs` and `src/state/gta6_worldview/stream_read.rs`, which both turn an entity into a `Node` with its parent, distance, level and radius |
| The model reader | `src/state/gta6_worldview/model.rs` and `src/state/gta6_worldview/texture.rs`; nothing for GTA V's world |
| The look | `crates/domain/or-world/src/state.rs` and the `Enabled` of the GTA VI index for the switches; the viewer's sky, lights, wind and water files for the frame |
| The edit store | `crates/domain/or-maps/src/operation.rs` with its session for GTA V; the three entity edit files of `src/state/` over `crates/io/or-io/src/gta6/map_grow.rs` for GTA VI |
| The selection's instance | The GTA V instance of `crates/domain/or-depends/src/world_forest.rs` carries its node, map and entity; the GTA VI one does not and gains the node in R71c |

Four things changed from the outline that was agreed, each forced by code:

- **A title is opened into an install.** A reader needs state for each worker (open files, open packs, keys), and the three mounts share no type, so the trait that is one static value for a game cannot also be the thing that reads.
- **The editor leaves the title's trait.** GTA V's store is `or-maps`, which depends on the project and on the GTA V adapter; the adapter cannot return it without a ring. The registry line pairs the two, and GTA VI's store is in its adapter.
- **Interiors have a slot of their own in a map body.** GTA V inserts an interior's entities into the forest under the instance (`interior_key`); GTA VI keeps them out and draws them by its room and portal walk. The default is the first, and a title that walks portals fills the slot.
- **A model carries the game's vertex streams and declarations**, with a surface for the fallback beside them. Today's CPU model is one fixed vertex layout that only OpenRage's own shader reads, and a game's program reads its own.

What stays out of the contract:

- **The mount and the address of a body.** An id is a row number; what it stands for (a logical path, an archive and an offset) is the adapter's.
- **The records an edit changes.** The core holds an operation's name, address, guard and inverse as an envelope; the record inside is the title's, on the title's own file.
- **The title's day.** The core's frame facts are what the host and the fallback need: the sun's direction for the cascades, an ambient and a key light, an exposure, the clear colour, and lights as point, spot and capsule. Everything else of a title's day reaches its own programs through the feed, by name.
- **Far stand-ins, grass, water meshes, time archetypes, world-state selectors.** They are a title's extras in slots.
- **The project.** The core does not depend on `or-project`; a finding of a core rule is the core's own small row, which the caller that has a project maps onto the row of `or_project::content::rule`.

## How a game differs

Shared code holds no test of which game it serves. A game differs at three levels, and a test fails when the core, the host or the app's world modules name one ([the developer's experience](#the-developers-experience)).

### Capabilities, as data

A capability says what the game has and what it can write back; a panel section, a tool or a verb whose capability is absent is hidden or greyed with the reason.

| Capability | GTA V | GTA VI | RDR2, expected |
|---|---|---|---|
| A ground and collision query | yes: `crates/domain/or-world/src/collision/` over the bounds `crates/domain/or-depends/src/world_collision.rs` streams | no: the ground is the install's height map | unmeasured |
| Placement rows and find | yes: `crates/domain/or-depends/src/find.rs` | rows for one archive, in the Map screen's store; no search | no |
| A weather gate on what streams | yes | none found | unmeasured |
| A level of DLC, story or multiplayer | yes | modes and script-managed groups instead | unmeasured |
| An edit store | yes, in a project | yes, over a document | no |
| A base to compare with, put back to and merge against | yes | no | no |
| Its own programs | Legacy after R71f, Enhanced after R71g | after R71h | no |

### Policies with defaults

A number is a field of the stream policy and a rule is a function it holds; the core writes the default and a game overrides only what it disagrees with. The cases the tree already shows:

- **When a parent waits for its children.** A GTA V parent waits for exactly the children it states (`with_exact_children`); a GTA VI parent's count is raised to those loaded. Today that is a flag on the forest.
- **When a map streams in.** GTA V's rule is its box grown by `PRESTREAM` metres, taking the entities' box where the streaming one does not hold it (`streams_in`); GTA VI's is a sphere and a tier distance (`MapBox::wants`). Today one is in the core and one outside it.
- **How a level fades.** `openrage-cli world select` fades every level over one distance; GTA VI reads its fade distances from the install.
- **How many parent maps a map has.** One for GTA V, a list with a selector for each element for GTA VI.
- **What a node's category scales.** GTA VI reads a category from load flags and scales by tables; GTA V's nodes are all of one category, so no table runs.

`crates/domain/or-world/src/streaming_rules.rs` and the constants of `crates/domain/or-world/src/interior_portals.rs` are GTA VI's build rules and move to its adapter; the portal walk itself is geometry and stays, for any title that fills the slot. The seven levels of `Level` stay the core's ladder: both titles use them, and whether RDR2's ladder differs is unmeasured until R71k reads its maps.

### Extensions in slots

The core calls a slot and does not know who filled it:

- **Layers.** GTA V: road nodes, navmesh, scenario points, collision by material, occluders, parked vehicles (the layers of [R70f](Roadmap.md#r70f-layers-of-what-the-game-hides)). GTA VI: the lane graph, map lights, places with their outlines, the occluder tier.
- **Operations.** GTA V: the renumbering of a parent map's links, the manifest, the first edit that imports. GTA VI: the selector of a parent map for each element, prop and grass lists.
- **Rules.** GTA V: the rules of `crates/domain/or-maps/src/rules.rs`. GTA VI: the link and child-count check the command line runs today.
- **Panel sections.** GTA VI: its modes and script-managed groups, with the choice between what the data proves and the game's own groups. GTA V: the level of DLC and the mode.
- **Streamed extras.** GTA VI: far stand-ins as cards, grass, batch levels, time archetypes, the room and portal visibility.

A sentence a slot shows a person is a key the adapter returns and the app's catalogs hold, through `tr_key!`, so an adapter carries no translated text and the language gates still read every key.

### What RDR2 is expected to need

Nothing here is measured. Expected from what the archive lists: its own index rows and mount, a model reader for resources of its container, and a policy that starts from the defaults. Whether it has interiors by portal walk, lists, a weather gate or a ladder of its own is what R71k's first part answers, and a need the contract cannot state is a change to the contract made there, with the kit as its check.

## Drawing

The owner chose option C: one shared frame host in which each game runs its own shader programs, with a plain fallback for what cannot be translated.

### The host

The host is in `or-renderer` and owns the mechanics once: device pages and the texture pool, instancing, culling and occlusion, the streaming budget, render targets and the scheduling of passes, picking and read-back, and the editor's overlays. The overlays (the manipulator, the selection outline, the layers, collision wireframes) draw in a neutral pass after the game's frame, which the overlay pass already is, so they work for every title and in the fallback.

Most of it exists and is moved, typed on the core's model in place of GTA VI's: the pages, pool, layout, occlusion, budget and readiness of the world files of `src/render/viewport/`, and the instances and batches of `crates/runtime/or-renderer/src/world.rs`. What does not exist is pages that hold a game's own vertex layout, a pool bound by a game's sampler names, an id pass to pick by, and a scheduler of declared passes.

### The render profile

A profile is what a game states so that the host can run its programs. It is data and pure functions in the adapter, with no type of the graphics library:

- **Where its programs come from** in the install, the translator that lowers them, by name, and the key a lowered program is cached under.
- **What it needs of the device**, in the shape `device_needs` of `crates/runtime/or-renderer/src/game_shaders/meshlets.rs` has, so that a device that lacks it gets the fallback and the reason.
- **Its frame**: named targets with their formats, and a list of passes over them, each of a kind the host knows.
- **How a material binds**: which programs draw it in which pass, with the blend, depth and cull state the pass needs, or a refusal with its reason.
- **Its feed**: each frame, the values of its constants, by reflection name for GTA V and by binding and name hash for GTA VI, whose programs carry no names.

What each game states, from today's code:

| | GTA V | GTA VI |
|---|---|---|
| Program source | the effect files of Legacy; the two containers of Enhanced, which no bake reads | the effect containers in the install's archives |
| Translator | the vendored C++ translator in a helper process, then the legalizer; the Rust front end for Enhanced's bitcode | the Rust front end for the console's native code, in the process |
| Geometry | a vertex buffer laid out by the vertex stage's inputs | the file's vertex data and meshlet records in storage buffers, one instance a meshlet |
| A material | variables and samplers by name | a record of words and a heap of textures by number |
| Targets | four colour targets, a depth and stencil with material ids, a float colour | up to four from the pixel program, a float depth, no stencil |
| Feed | `EngineConstants` and `Lighting`, by name | bytes for each binding by role; what only the game fills is left at nought |

⚠ A list of passes is not enough to state today's GTA V frame, so "data, not code" is a target with a counted exception. The passes `render_deferred` schedules build a dome mesh, a dual-paraboloid sky map, cascade fits, light volumes as instanced spheres and room masks on the CPU, and its post passes are picked by number. The host therefore has a small set of pass kinds (surfaces by bucket, a shadow cast, a pass over the whole target, instanced light volumes, a copy, a mip chain, a meter), a profile's mesh for a pass is data the adapter makes, and a pass no kind states is a named pass in `or-renderer`, in a module named for its title, counted by a ledger a test holds. How many kinds and how many named passes GTA V's frame needs is unmeasured: R71f's first part restates that function as a list and counts them, and no line that touches the device enters an adapter whatever the count ([ruling 5](#asked-of-the-owner)).

The translators stay where they are. A domain crate may depend on neither `or-shader-front` nor `or-cli`, so the profile names its translator and the app hands the host one for each name, as `src/state/gta6_shader_translator.rs` and the helper process of `src/state/game_pack.rs` do today.

### The fallback

What cannot be translated, or what a device cannot build, is drawn for that material in the fallback pass, and the panel lists what fell back and why. The fallback is the host's own shader over a small surface a model states for each mesh: a colour map, a normal map where there is one, how its alpha is used and a tint.

The GTA VI world is that pass today and nothing else. Its families are a hand port of what its programs do, which is the rejected option A already built: it is kept as it is, it gains no family from the day this plan lands, and R71h replaces it material by material. GTA V's fallback surface is the small one above, and a per-shader port of GTA V's materials into the fallback is not built: that work is the profile's.

Until R71h the GTA VI world also borrows from another title: its lighting comes from a pack baked from a GTA V Legacy install, and with none on disk it is the plain forward pass. The architecture test lists that borrow by name ([ruling 4](#asked-of-the-owner)).

### What the graphics library cannot run

- **Hull, domain and geometry stages.** wgpu has none on any backend. GTA V's tessellated and instanced technique variants bind them, so a city cannot use the game's own instanced variants; the plain techniques a material draws with build almost whole ([measured](Shader_Pipeline_Design.md#what-a-pass-needs)). A material whose only technique needs such a stage falls back.
- **GTA V's instancing.** The game's passes draw one drawable today: a vertex buffer for each surface built on the CPU, one world matrix a scene, and decoded textures. R71f measures the two ways to place an instance (a draw for each with its constants, or a lowering that reads the transform by instance) before it builds one.
- **GTA VI on Metal.** Its translated passes are built and drawn on Vulkan and on Direct3D with DXC; on Metal they are written and validated as text and no device has built one ([GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md)). The owner's machine is a Mac, so R71h begins there.
- **GTA VI's lighting programs.** Only the surface pass of one model runs GTA VI's code; its lighting and post are OpenRage's shaders written after the build's. Why its own are not run is not checked, and R71h's first part is that question.

### Where translated programs live

Translated programs come from the user's own install, are cached in the configuration folder and never enter the tree, the policy the keys follow. Two things change:

- **The cache is keyed by game and install.** Today the GTA V pack is one folder for the machine, rebaked when the install's fingerprint changes, so a second install or a second title overwrites the first.
- **The policy becomes a gate.** The provenance lint's list of game content extensions names the effect files and not a translated module, and nothing ignores a pack folder, so a module committed by mistake would pass the hook. R71f adds the extensions of every cache to the lint and to the ignore file.

### The alternatives rejected

| Option | What it is | Why not |
|---|---|---|
| A | OpenRage's own shaders, each game translating its materials into them | An approximation that drifts and is never the game's look; it is what the GTA VI world does today, and it survives only as the fallback |
| B | Each game owns its whole renderer | Everything the host does is written once for each game |
| D | The game's shaders handed to the native API | Direct3D only, so not on Linux or macOS |

## The app

The app has one world, three times over:

- **One session**, in `src/state/`, keyed by game and install, held by a handle on the flat `AppState` (`the_shell_is_flat_rather_than_deep`). It owns the opened install, the camera, the switches, the world thread and the reader pools. No static holds a world: the viewer's statics become fields of a session or of the opened install, and the three that are single-valued for the process are the first to go. A world tab draws its session, so an install's tab and a project's tab draw two stacks at once, as [R70c](Roadmap.md#r70c-the-gta-v-world-streamed-and-drawn) asks.
- **One bridge**, in `src/render/viewport/`, that hands a session's selection and feed to the host and names no game.
- **One set of panels**, whose sections follow the capabilities and the slots. Of the World panel's sections today, Time and weather, Camera, Capture and Streaming are neutral; What streams becomes the switches as data (named toggles, grouped toggles, a flag for a group known by inference); Mount becomes the index's report as rows of a label and a value; and the blocked section's sentence is the adapter's key. The status bar and the panel take the game from the session, where they name GTA VI today.

Two kept shapes change and are read once in their old form: the World viewer's preferences, which store GTA VI's modes as a bitmask, and the kept indexes, which are rebuilt.

The command line reaches the same session type with no window: `openrage-cli world index`, `at`, `find` and `select` take any registered title, and a verb whose capability a title lacks refuses by name.

## The developer's experience

- **The conformance kit.** A macro in the core, in the manner of `declared_gates!` in `crates/io/or-io-verify/src/gates.rs`, writes the same tests for every adapter: an adapter's test file names its title and a fixture, and the macro writes the tests. The fixture is a stated scene (parents and children, a held map, a group at an hour, an interior, a list) the adapter builds in its own form. The kit has two tiers, because the GTA VI tests build rows and bodies and never an install: a tier over rows and bodies, which every adapter reaches, and a tier over an install written to a scratch folder, which GTA V reaches today through the fixtures of `crates/domain/or-depends/tests/`. Whether `or-io` can write a scannable GTA VI archive with its registration documents is unchecked; R71b measures it, and until it can the second tier is recorded as not attempted for GTA VI.
- **The fake title.** A public module of the core: a few maps, parents and children, a cube, a flat texture, a two-pass profile in the shader language the host reads directly, and a store that keeps its edits in memory. The core's tests prove streaming, selection, the kept file, the queries and the edit envelope on it with no game, and the self-check draws it. It is in no registry a person sees.
- **Architecture tests**, beside the gates of `src/doc_counts.rs`. The core's source, the host's world modules and the app's world session, bridge and panels name no game; no adapter's manifest lists another adapter, the renderer, a tool or the app; no static holds a world. Each test has a ledger of what still breaks it on the day it lands (the GTA V modules of the core before R71d, the GTA VI types of the bridge before R71c, the borrowed lighting pack before R71h), an entry that no longer breaks it fails the test, and R71l ends with every ledger empty.
- **What adding a game costs**: one adapter crate, one line in the registry, the kit's macro in its tests and its keys in the catalogs. A test holds that, as `crates/title/or-title/tests/adding_a_title.rs` holds it for a profile: the fake title goes through every path with no line of the core, the host or the app naming it.

Two borrows below the adapters are not faults and are listed where the test would see them: the GTA VI streamer reads its fade distances through the reader of a file GTA V also ships, and GTA V's kept file is named with a hash function that lives in the GTA VI module of `or-io`.

## The order of work

A row is one programme part, lands in the parts its brief lists, and ends on a check that failed once before it passed. A row starts when the rows it depends on have merged.

| Row | Content | Depends on |
|---|---|---|
| R71a | The contract, the kit and the fake title | |
| R71b | The GTA VI world behind the contract | R71a |
| R71c | One session, one host, panels by capability | R71b |
| R71d | The GTA V world behind the contract, and one kept file | R71a; the finished branches of R70 landed |
| R71e | The GTA V world drawn in the fallback pass | R71c, R71d |
| R71f | The render profile, and the programs of GTA V Legacy | R71e |
| R71g | The programs of GTA V Enhanced | R71f; the Enhanced inventory of [R58f](Roadmap.md#r58f-shader-baking-off-windows) |
| R71h | The programs of GTA VI over a world | R71c; R71f for the pass kinds |
| R71i | One edit stack | R71b, R71d |
| R71j | The world tools built once | R71c, R71e, R71i |
| R71k | Red Dead Redemption 2 | R71a; [R56](Roadmap.md#r56-rdr2-the-second-container) for the formats |
| R71l | Leftovers and the ledger | every row above |

R71b and R71d share no file once R71a has merged and run together; R71i runs beside the drawing rows; R71k's measuring part waits for nothing but R71a.

Rules for every row: no window is opened, a screen is checked on `just shots` and `just self-check`; documents travel with the code, among them [Crate_Reference.md](../reference/Crate_Reference.md) and [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md), whose cited paths the citation gate reads; a moved test keeps its body; and a format's read path changed by a move runs that format's slow suite.

### The contract, the kit and the fake title (R71a)

Read `crates/domain/or-world/src/streaming.rs`, `crates/domain/or-depends/src/world_forest.rs`, `src/state/gta6_worldview/stream.rs` and `src/state/gta6_worldview/stream_world.rs`, `crates/domain/or-depends/src/world_index/kept.rs`, `crates/io/or-io-verify/src/gates.rs` for the macro, and `crates/title/or-title/tests/adding_a_title.rs`.

- **Builds.** In `crates/domain/or-world/src/`: the contract and the neutral model of [the contract](#the-contract); the stream policy, with the forest's flag and the two stream-in rules as its fields; a driver around the forest (wanted maps and their parents, read, insert and remove, select, an instance for each node that draws, carrying its node) that takes an install's readers; the kept-file mechanism (a head of title, rules version and the stamp of each archive, a section of bytes for each archive that the adapter encodes, written beside and renamed, with a verdict of absent, current, changed, foreign or refused); the camera pose; the kit's macro and its first tier; the fake title; and the architecture tests with their ledgers.
- **Deletes.** `with_exact_children`, once its one caller passes a policy.
- **Check.** The fake title streams and selects through the kit: a parent draws until its children are loaded and the children after. The kit run against a fake built to break each rule fails by that rule's name. A line naming a game added to a core module outside the ledger fails the architecture test. The GTA V forest's suites and the 25 tests of `src/state/gta6_worldview/stream_tests.rs` at `9fdf0fee` pass with no body changed.
- **Must not touch.** The GTA V modules of `or-world`, `or-maps`, and the viewer beyond the one call that passes a policy.
- **Not known.** Whether the neutral driver can serve GTA VI's chunks, swaps and bands without a second driver: R71b answers by deleting its own, and the check there is that one caller of the forest's `select` is left outside tests.

### The GTA VI world behind the contract (R71b)

Read [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md), `src/state/gta6_worldview/mod.rs`, `select_asking` in `src/state/gta6_worldview/stream.rs`, `src/state/gta6_worldview/model.rs` with `meshes_at` of `src/render/gta6_drawable.rs`, and the shots steps of [Development_Workflow.md](../guides/Development_Workflow.md#screenshots).

It lands in four parts, each on its own check:

1. **The reference.** No World shots script is in the tree, and a comparison writes no reference. Write the script (fixed cameras, hours and modes, `world settle`, `world converge`, a shot and a `world dump` at each), run it twice on the unchanged tree and compare the two runs. Whether two runs on this machine agree is unchecked; if they do not, the picture check is void until they do, and the dumps alone are the check.
2. **The move.** The 20 files of the viewer that name neither the renderer nor the app at `9fdf0fee` move to or-world-gta6 with `git mv`, their tests with them: the index with its scan and its states, the stream reader, the far stand-ins, the grass, the census, the occluders, the material, the lights' index, law, levels and variables, the terrain grid and the water's fog and ground. The world thread and the reader pool go to the core. The three readers by member hash go to `or-io`. The mount binding follows once it takes the configuration folder and the key snapshot from the opening in place of the app's, and the index's kept file becomes the core's.
3. **The selecting half.** Two symbols it takes from the renderer move below it, then `src/state/gta6_worldview/stream.rs`, the interiors and the room walk move. `select_asking` is split: the mechanics every title shares (a chunk for each map and list, held parents as gated swaps, what a selection waits for) go to the core's driver, and GTA VI's rules (the held codes, time archetypes, batch levels that hide a parent's instances, cards that hand off to trees, the walk that marks an instance as casting only) stay in its adapter behind the policy and the slots. `crates/domain/or-world/src/streaming_rules.rs` moves to the adapter but for the two functions the forest calls.
4. **The outputs.** The model reader returns the core's model, with the part of `src/render/gta6_drawable.rs` it needs moved below the app; the texture reader returns a block format of the core's; the sky, lights, wind, water and terrain files return the core's lights and frame facts and feed the rest by name. The conversion to the renderer's types is in the renderer, in its GTA VI module.

- **Deletes.** `src/state/gta6_worldview/index_keep.rs`; the GTA VI streamer's own loop around the forest; the block that opens an archive for a worker, written once in the adapter in place of six times.
- **Check.** The World shots and dumps of part 1 match after each part. The tests of the viewer are counted before and after: of the 189 at `9fdf0fee` none is deleted, those of part 2 change in their imports alone, and the pull request lists each test that part 3 or 4 restates. The kit's first tier passes for GTA VI.
- **Must not touch.** What the picture is: no family, pass or constant changes. The Map screen's GTA VI files, the model viewport's GTA VI path, and the singleton, which stays in the app as a shell over the crate until R71c.
- **Not known.** Whether `or-io` can write a scannable archive for the kit's second tier; the share of the viewer's tests that part 3 restates; and the order in which the files of part 2 compile in the crate, since one that imports a sibling still in the app waits for it.

### One session, one host, panels by capability (R71c)

Read `src/render/viewport/world.rs` and its neighbours, `crates/runtime/or-renderer/src/world.rs`, the five world viewer panel files, `src/self_check/shots/world.rs` and `src/ui/panels/status_bar.rs`.

- **Builds.** The session of [the app](#the-app), and `viewer()` gone from every call site. The host: the pages, pool, layout, occlusion, budget and readiness of the bridge moved to `or-renderer` and typed on the core's model; the selection's instance carrying its node; the bridge reduced to handing a session to the host. The panels by capability and slot. The neutral preferences, the old shape read once. The shots steps addressed to a session. The self-check drawing the fake title's world, which needs the renderer's test packs made from its own shader language to be reachable outside its tests.
- **Deletes.** The static viewer and the three process-wide values; the bucket and family enums of the app, stated once in the renderer.
- **Check.** The World shots and dumps of R71b match. `just self-check` with its viewport draws the fake title's world and fails when it paints nothing. Two sessions opened in one process do not share an exposure or a night map, in a test that fails on the static. The architecture test's ledger for the bridge and the panels is empty. The French screens of the world hold at the smallest window.
- **Must not touch.** The frame: `render_deferred` is still the one frame, and the fallback's families are unchanged.
- **Not known.** Whether overlays drawn after GTA VI's temporal pass carry the frame's jitter; a shot with a line over a still world answers it.

### The GTA V world behind the contract, and one kept file (R71d)

Read the header of `crates/domain/or-world/src/lib.rs`, `crates/domain/or-depends/src/world_index.rs` with its folder, `crates/domain/or-depends/src/world_forest.rs`, and the manifests of `or-depends` and `or-maps`.

- **Builds.** or-world-gta5: the GTA V modules of `or-world`, moved with `git mv`; the world half of `or-depends` with the catalogue, the tables and the contents its index imports; the script forms of `crates/domain/or-maps/src/script_text.rs`, which `find` writes with, and the filename rule of an area's bounds that the catalogue calls in `or-maps`. Its title: the indexer over the mount, the map reader of `crates/domain/or-depends/src/world_forest/read.rs`, its policy, the state gate as its switches, its interiors in the forest, its capabilities. The core takes the place parser, the manipulator, the box scene and the pose, which ends the edge from `or-depends` to `or-maps`. The registry crate, with its line for each title. `openrage-cli world index`, `at`, `find` and `select` for any registered title.
- **Deletes.** `crates/domain/or-depends/src/world_index/kept.rs` and the GTA V driver around the forest, in favour of the core's; the include by path of `or-depends`' fixtures into `or-cli`.
- **Check.** The kit passes for GTA V in both tiers. The install suites that hold [R70b](Roadmap.md#r70b-the-gta-v-world-index), R70c, R70e and R70g pass with no body changed, and a second start still opens no archive that did not change. The world verbs answer over the fake title in the default tier and over a GTA VI install in the slow tier, and `ground` over GTA VI refuses by name. One world kept-file format is left in the tree.
- **Must not touch.** What a GTA V file reads or writes as. The move is one commit made on `main` between waves, with no other lane open on the three crates, because the local branches of R70 edit them.
- **Not known.** Which of the two GTA V deletions with renumbering survives; R71l settles it from which verbs still reach the first.

### The GTA V world drawn in the fallback pass (R71e)

Read `crates/runtime/or-renderer/src/world.rs`, `crates/runtime/or-renderer/src/game_shaders/drawable.rs` for how a GTA V drawable becomes surfaces, and [the Gen8 and Gen9 section](../reference/Crate_Reference.md#gen8-and-gen9-enhanced-is-a-separate-target) of the crate reference.

- **Builds.** GTA V's model reader: a drawable, a drawable dictionary's entry and a fragment to the core's model, for both targets, each by its file's own RSC7 version; its textures as the blocks the file stores; the fallback surface of a material by its sampler names. The World room for a GTA V install, read only.
- **Deletes.** Nothing.
- **Check.** A synthetic world of parent and child levels draws the level the rules select at each distance, in real frames with no window. Over each install, at fixed cameras, the instances drawn are the instances `openrage-cli world select` lists there, and every model not drawn has a named reason. The frame over the city is measured on the owner's machine and its budget recorded. The contract is frozen here.
- **Must not touch.** The Shaded viewport of one drawable; GTA VI's picture.
- **Not known.** The frame's cost and the memory of the city's textures: both unmeasured, and this row measures them before R71f builds on it.

### The render profile and the programs of GTA V Legacy (R71f)

Read [Shader_Pipeline_Design.md](Shader_Pipeline_Design.md), `crates/runtime/or-renderer/src/viewport/game_frame.rs`, `crates/runtime/or-renderer/src/game_shaders/mesh.rs` with its scene, and `src/state/game_pack.rs`.

It lands in three parts:

1. **The frame as a list.** Restate `render_deferred` as targets and passes of kinds, count the kinds and the named passes it needs, and record both. Give a pass of the pack's manifest its render state, which the renderer chooses today. Key the pack by game and install. Add every cache's extensions to the provenance lint and the ignore file.
2. **An instance of a game's program.** Measure a city view drawn with a draw for each instance against a lowering that reads the transform by instance, and build the one the measure favours. Lay the pages out by the vertex stage's inputs. Bind the streaming pool's textures by sampler name.
3. **The world through the profile**, with GTA V's day from the install's timecycle as its feed, which is the feed [R70d](Roadmap.md#r70d-the-day-sky-water-and-shadows-of-the-world) asks for.

- **Deletes.** `render_deferred`, once the list draws its frame; the one pack folder for the machine.
- **Check.** The frame drawn from the list equals the frame `render_deferred` drew, for a drawable and for the GTA VI world, before that function is deleted. A drawable in Shaded and the same drawable placed alone in the world at the same hour and weather agree within a recorded tolerance. With no pack the world draws in the fallback and says so, and the panel lists each material that fell back with its reason. A translated module staged in a commit fails the hook.
- **Must not touch.** The helper's protocol; no C++ enters the editor.
- **Not known.** The count of pass kinds and named passes; the cost of either way to place an instance. Both are the first measures of the row.

### The programs of GTA V Enhanced (R71g)

Read [Shader_Definition_Database.md](../features/Shader_Definition_Database.md) for the two containers and `crates/tools/or-shader-front/src/dxil/`.

- **Builds.** A bake that drives the Rust front end over Enhanced's containers into a pack of the same manifest, and Enhanced's profile over it.
- **Deletes.** Nothing.
- **Check.** The inventory R58f leaves open is recorded. The share of Enhanced's programs that lower and that each backend's writer takes is measured and recorded. Over an Enhanced install the world draws through its own programs, and what does not is listed with its reason.
- **Must not touch.** Legacy's pack and bake, which stay as they are, and the helper's protocol.
- **Not known.** Everything: no bake has translated an Enhanced program. Until this row closes an Enhanced install draws in the fallback and the panel says why.

### The programs of GTA VI over a world (R71h)

Read [GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md), `crates/runtime/or-renderer/src/game_shaders/meshlets.rs`, `src/render/gta6_meshlets.rs` and `src/state/gta6_shaders.rs`.

- **Builds.** First the measures: the share of the install's programs that translate and bind (`openrage-cli rpf scan-rdna-bind`), whether a Metal device builds a pass, and why the build's lighting programs are not run. Then GTA VI's profile: its surface pass over the host's pages as storage buffers, a transform for each instance, which nothing feeds today, and the bindings only the game fills, named. Then its lighting, by its own programs where the measure allows and by named passes where it does not.
- **Deletes.** A family of the fallback's shader once the profile draws every material that used it; the app's mapping of a material onto a family with the last of them.
- **Check.** A model drawn alone in Shaded and placed alone in the world agree within a recorded tolerance. A map's materials draw through their own programs, and each that falls back is listed with its reason. The borrowed GTA V pack is gone from the ledger, or [ruling 4](#asked-of-the-owner) says it stays.
- **Must not touch.** The fallback's families, which are deleted only family by family as the profile replaces each.
- **Not known.** Whether this is possible on the owner's machine at all.

### One edit stack (R71i)

Read `crates/domain/or-maps/src/operation.rs` with its session, `crates/domain/or-maps/src/changes.rs`, `src/state/workbench/maps.rs`, and the three GTA VI entity edit files of `src/state/` with `crates/io/or-io/src/gta6/map_grow.rs`.

- **Builds.** In the core: the envelope (an operation's name, its address, the guard it states, its inverse), the session's order, undo and redo, and the skeleton of comparison, put-back and merge over a trait that gives an entity's identity (its guid, its archetype, its pose). GTA V's store stays `or-maps`, its operations registered through the slot and its record, its digest and its project untouched. GTA VI's store is new, in its adapter: place, move, turn, scale, retype, add and remove across linked bodies, as operations with inverses over the sections of a body, guarded by a digest of that body. The Map screen keeps one history.
- **Deletes.** The three GTA VI entity edit files of `src/state/` (2,162 lines at `9fdf0fee`) and `src/state/gta6_map_extents.rs`, and the second history of the Map screen.
- **Check.** The kit's edit tests pass for both titles and the fake: every operation followed by its inverse leaves the store byte for byte, a stale guard refuses, and an operation with no inverse fails a test. The tests of `or-maps` pass with no body changed. The GTA VI edit tests are restated with no `AppState`, none deleted.
- **Must not touch.** The navmesh and collision edits of GTA VI, which are edits of a cell and a fragment with no counterpart in the vocabulary and stay document steps ([ruling 6](#asked-of-the-owner)).
- **Not known.** Whether a scanned body can serve as GTA VI's base for a comparison; until it does, GTA VI's capability says no.

### The world tools built once (R71j)

Read [World_Editor_UX_Design.md](World_Editor_UX_Design.md), `crates/domain/or-maps/src/gizmo.rs`, `src/render/viewport/pick.rs` and `src/ui/commands/target.rs`.

- **Builds.** The surface the screen rows of R70 stand on, for every title: the pick by an id pass, the selection outline and the manipulator in the overlay pass, the registry of layers and of panel sections, a command's capability in the command table, and placement rows as a capability. GTA VI's placement rows replace the Map screen's world store and its kept file.
- **Deletes.** `src/state/gta6_world.rs` and `src/state/gta6_world_fill.rs`.
- **Check.** On the fake title a click returns the node under the pixel, a drag moves it as one operation of R71i and Undo takes it back, in real frames with no window. A tool whose capability a title lacks is greyed and says why. The plan overlay of a GTA VI archive draws the same shots as before.
- **Must not touch.** What each tool does for a mapper, which the rows of R70 own, and the Map screen of a GTA V project beyond its pick and its overlay.

### Red Dead Redemption 2 (R71k)

Read [R56](Roadmap.md#r56-rdr2-the-second-container) and `crates/io/or-io/src/rdr2/`.

It lands in four parts: the measures (what its map, type, model and bound files are, how its archives are ordered, what its shader containers hold); a mount in `or-title`; the formats in `or-io`, which R56 gates; and the adapter, view only, in the fallback pass.

- **Deletes.** Nothing.
- **Check.** Each measure is recorded with its command and revision. The adapter passes the kit's first tier, and an install draws with every model not drawn named.
- **Must not touch.** GTA V's mount: RDR2's is its own, and the function that mounts GTA V is not widened to serve it.
- **Not known.** Every figure. R56 warns that each estimate made there before a format was measured was low, and this row gives none.

### Leftovers and the ledger (R71l)

- **Builds.** Nothing new. It settles the two GTA V deletions, moves the conversions the renderer still holds for a title into that title's feed where a profile now serves them, and corrects every document that states the old shape.
- **Deletes.** Whichever of the two GTA V deletions loses, and every ledger entry.
- **Check.** Every ledger of the architecture tests is empty, and an entry added to one fails. The lines each duplicate of [what is written twice](#what-is-written-twice) lost are published as a table measured with `git diff --shortstat` between `9fdf0fee` and the closing revision.

## What it changes for R70

What is finished of [R70](Roadmap.md#r70-the-gta-v-world-editor) lands as it is. The headless parts keep their checks and move crate in R71d; the screens are built once on the core, for every title that has the capability.

| Row | What changes |
|---|---|
| R70a | Nothing |
| R70b | The index lands as built, then moves to the GTA V adapter and onto the core's kept file in R71d |
| R70c | Its seam, its viewer for each tab and its feed are R71c, R71d and R71e; it keeps the room, the tab, the bar of the stack, the camera's inputs and the presets |
| R70d | The day and the water reach the world through GTA V's profile in R71f; the water reader is unchanged |
| R70e | The headless find moves in R71d; its screen is built on the session, for any title with placement rows |
| R70f | Each layer is a slot a title fills, on the registry of R71j |
| R70g | The geometry stays in the core; the query is a capability GTA V has |
| R70h | The vocabulary stays GTA V's store; its envelope, session order and undo are the core's in R71i |
| R70i | The pick, the outline and the manipulator are R71j's, for every title |
| R70j, R70k, R70l, R70m, R70n, R70o, R70p | Their tools draw in the overlay pass and call operations registered through GTA V's slot; their data and rules are unchanged |
| R70q, R70r, R70w, R70x, R70s, R70u | Nothing but paths: they build, scan, merge and send what the store holds |
| R70t | The walk asks the core's collision query |
| R70v | The settings of a world follow the capabilities |
| R70y | The comparison's skeleton is the core's in R71i and its panel is built once |
| R70z | The browser's grid is built on the session; vehicles stay GTA V's |

Three statements of [World_Editor_Plan.md](World_Editor_Plan.md) are replaced: that the seam between the titles is declared in the app beside the GTA VI viewer, that a GTA VI world keeps its own World panel, and its ruling 9, which left drawing the map through the game's shaders out of the first cut and which the owner's choice of option C answers.

## Risks

- **The move breaks the picture.** R71b compares shots and dumps made before it, part by part, and changes no family, pass or constant.
- **The contract is wrong.** It is drawn from two titles, frozen only when both draw, and proved on a fake that shares code with neither.
- **A third driver is written and the two stay.** R71b and R71d each end with their own deleted, and one caller of the forest's `select` outside tests is the check.
- **The local branches of R70 conflict with the move.** At `9fdf0fee`, 40 local branches of R70 hold commits `main` lacks. R71d's move is one commit on `main` between waves, after the finished ones land ([ruling 7](#asked-of-the-owner)).
- **A list of passes cannot state a game's frame.** Named passes in the renderer, counted, and no device code in an adapter.
- **The city costs more than the frame has.** R71e measures in the fallback before R71f builds the game's passes on it, and R71f measures an instance before it chooses how to place one.
- **GTA VI's programs do not build on the owner's machine.** R71h measures first; the fallback is the picture until then, and stays on a device that cannot.
- **RDR2 is further than a format away.** No mount, hash names, one format declared. R71k measures before it promises.
- **Game content enters the tree as a translated module.** The lint and the ignore file of R71f.
- **A capability becomes a test of the game by another name.** A capability states what a title has; a branch on it that changes what a shared rule does is a policy and is written as one.

## Rulings

### Made on 2026-11-26

- **The shape**: a core and one adapter crate for each game ([recorded](Project_Context.md#the-world-viewer-and-editor-become-one-core-and-an-adapter-crate-for-each-game-2026-11-26)).
- **Who lifts the GTA VI viewer**: this programme, with the other GTA VI sessions paused meanwhile ([recorded](Project_Context.md#this-programme-lifts-the-gta-vi-viewer-out-of-the-app-and-the-other-gta-vi-sessions-pause-2026-11-26)).
- **The drawing**: option C ([recorded](Project_Context.md#each-game-draws-its-world-with-its-own-shader-programs-in-one-shared-frame-host-2026-11-26)).

### Asked of the owner

Each has a default a row takes until a sentence reverses it.

1. **A registry crate lists the titles**, depending on every adapter, where the agent guide says there are no facade crates. Default: yes; it re-exports nothing and is what `or-content` is for the kinds of content.
2. **`or-renderer` depends on `or-world`**, which the plan of 2026-11-25 said it does not. Default: yes; the edge points down the chain and the host needs the core's model.
3. **The translators stay tools and the app hands them to the host.** The alternative moves the Rust front ends into a crate of the io layer, so that a caller with no app can translate. Default: they stay.
4. **The GTA VI world keeps GTA V Legacy's lighting pack until R71h.** Ending the borrow sooner changes the picture. Default: it stays, listed in the ledger.
5. **No line that touches the device enters an adapter**, so a game is one crate. Should R71f's count show that a game's frame cannot be stated as kinds and named passes, the owner is asked before a second crate for a game is made. Default: one crate.
6. **GTA VI's navmesh and collision edits stay document steps**, outside the world's edit stack. Default: yes.
7. **The finished branches of R70 land before R71d's move.** Which are finished is the owner's to say. Default: R71d waits, and R71a, R71b and R71c do not.
8. **GTA VI gains no comparison, put-back or merge in this programme.** Default: yes; its capability says no until a base is built.
9. **RDR2 is view first and edit later.** Default: yes, as proposed.
10. **The other GTA VI sessions resume on the viewer when R71c merges.** The owner paused them "meanwhile" and did not say until when. Default: until the app holds one session, since every row before that moves the files they edit.

## Sources

- The three readings of the tree at `9fdf0fee`, of 2026-11-26: the GTA VI viewer, the GTA V world and the core, and the renderer.
- [World_Editor_Plan.md](World_Editor_Plan.md) and [R70](Roadmap.md#r70-the-gta-v-world-editor) for the GTA V programme this stands beside.
- [Shader_Pipeline_Design.md](Shader_Pipeline_Design.md), [Shader_Definition_Database.md](../features/Shader_Definition_Database.md), [GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md) and [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) for what runs of each game's programs.
- [Crate_Reference.md](../reference/Crate_Reference.md#layers) for the layers and the edges.
