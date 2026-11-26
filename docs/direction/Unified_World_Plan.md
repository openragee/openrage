# Unified world plan

How the world viewer and the world editor become one title-neutral core with one adapter crate for each game, what the tree holds today, the contract between the two, how a world is drawn, and the order and the briefs of the rows that build it.

Status: the plan of 2026-11-26, written on the owner's decisions of that day ([the shape](Project_Context.md#the-world-viewer-and-editor-become-one-core-and-an-adapter-crate-for-each-game-2026-11-26), [who lifts the GTA VI viewer](Project_Context.md#this-programme-lifts-the-gta-vi-viewer-out-of-the-app-and-the-other-gta-vi-sessions-pause-2026-11-26), [the drawing](Project_Context.md#each-game-draws-its-world-with-its-own-shader-programs-in-one-shared-frame-host-2026-11-26)) and mended after its review of 2026-11-27; [R71](Roadmap.md#r71-one-world-for-every-title) says where each row stands.

## The shape in brief

**One core, `or-world`, that names no game, and one adapter crate for each of GTA V, GTA VI and Red Dead Redemption 2**, with one world session in the app, one frame host in the renderer, and each game's own shader programs run inside that host. Four facts of the tree shape the rows:

1. **A world is a stack whose level is fixed when it opens**, so the contract opens a world, never a folder, and an edit store is opened the same way.
2. **Lifting the GTA VI viewer is not a pure move**: by the closure of their imports at `9fdf0fee`, 13 of its 39 source files can leave the app before the selecting half.
3. **GTA V answers the contract early, with no file moved**: its adapter is first a crate over the crates as they stand, and the move is a later, mechanical row.
4. **The GTA VI world runs none of GTA VI's programs, and nothing draws the GTA V world.** The first is kept as GTA VI's fallback; the second is new work.

## What it rests on

- **The tree at `9fdf0fee`**, read on 2026-11-26 and by two reviewers on 2026-11-27: every line count here is `wc -l` at that revision and every test count the `#[test]` attributes a `grep` finds there. Nothing was built or run for the readings.
- **The owner's three decisions of 2026-11-26**, and [World_Editor_Plan.md](World_Editor_Plan.md), the plan of [R70](Roadmap.md#r70-the-gta-v-world-editor), three of whose statements this plan replaces ([what it changes for R70](#what-it-changes-for-r70)).
- **What is not known is said so**, in the row that needs it, with the measurement that would settle it.

## What the tree holds today

### GTA VI

The app holds two GTA VI worlds, each a process-wide singleton with its own index and kept file: the World screen streams and draws an install, and the Map screen lays a plan of an archive over its scene and edits a scanned map body as a document. Lines and tests at `9fdf0fee`:

| Part | Where | Files | Lines | Tests |
|---|---|---|---|---|
| The World screen's viewer | `src/state/gta6_worldview/` | 55 | 18,010 | 189 |
| The bridge that draws it, which is the host today | the world files of `src/render/viewport/` | 13 | 4,651 | 49 |
| Its panels | the world viewer files of `src/ui/panels/` | 5 | 2,211 | 13 |
| The Map screen's world store | `src/state/gta6_world.rs`, `src/state/gta6_world_fill.rs` | 3 | 1,765 | 9 |
| Map edits as steps of a document's undo | `src/state/gta6_map.rs` and its neighbours | 8 | 4,399 | 23 |
| Navigation and collision edits | the navmesh files of `src/state/` and `src/state/gta6_collision_edit.rs` | 7 | 2,941 | 26 |

What matters of it for this plan, each at `9fdf0fee`:

- **The viewer is one static.** `viewer()` in `src/state/gta6_worldview/mod.rs` locks a `LazyLock<Mutex<Viewer>>`, and 23 call sites in nine files reach it, 10 of them in `src/self_check/shots/world.rs`. The folder holds 15 statics, five of them single-valued for the process and wrong for two sessions: in `src/state/gta6_worldview/sky.rs` the exposure, the sky radiance table with its worker's slot and the one-key slot of the moment's worker, and in `src/state/gta6_worldview/sky_night.rs` the night maps of whichever install was taken last.
- **The viewer is declared inside the Map screen's store**, by path in `src/state/gta6_world.rs`, and 21 files import it through that module.
- **The map index and the stream path have no test over an archive**; their tests build rows and bodies by hand. Two tests of `src/state/gta6_worldview/texture_tests.rs` do read one: they write a resource with `write_rpf9_resource`, copy it into a scratch install, scan it and read a texture back.
- **The selection carries no identity.** An instance is a model, its rows, its sphere, a fade, a tint of three floats, two ambient factors and whether it only casts a shadow: no node, map or entity. Nothing picks, selects or moves in the World screen.
- **A height of the ground is answered**, from the install's height map, by `ground_under` in `src/state/gta6_worldview/stream_grass.rs`.
- **Every GTA VI edit is a step of a document**, on the sections of a scanned body, with no named operation, staleness guard, base or project.
- **Three readers of a body by member hash live in the app**: `src/state/gta6_worldview/index_types.rs`, `src/state/gta6_worldview/lights_lod.rs` and `src/state/gta6_worldview/lights_index.rs`.
- **The index is whole-install tables**, several derived across archives, which `src/state/gta6_worldview/index_keep.rs` writes whole under a head that also holds a word of `fresh_game.json`, a file a person writes beside the index.
- **The look is read below the adapter's layer**, in `crates/title/or-title/src/`, and the entity edits and link renumbering are in `crates/io/or-io/src/gta6/map_grow.rs`.

### GTA V

GTA V's world is headless: `openrage-cli world` reaches it and no screen draws it. Lines at `9fdf0fee`:

| Part | Where | Lines |
|---|---|---|
| The world model: maps, types, extensions, groups, manifests, state, creation, and the conversion of a bound to shapes | `crates/domain/or-world/src/` but for the streaming files, the transform, the portal walk with its tests, and the collision files other than `crates/domain/or-world/src/collision/bound.rs` | 12,996 |
| The index of an install, kept on disk, and its mount source | `crates/domain/or-depends/src/world_index.rs` and its folder | 1,910 |
| The streaming driver and its map reader | `crates/domain/or-depends/src/world_forest.rs` and its folder | 816 |
| Collision near a position, find, placements | `crates/domain/or-depends/src/world_collision.rs`, `crates/domain/or-depends/src/find.rs` and its folder, `crates/domain/or-depends/src/placement.rs` | 1,451 |
| The edit vocabularies, their session, the first edit, delete, compare, merge, rules | `crates/domain/or-maps/src/` | 22,178 |
| The command line's world verbs | the world files of `crates/tools/or-cli/src/` | 6,953 |

- **`or-world` is GTA V's model, not a neutral core.** Its header states two invariants a core must respect: nothing there copies the tree a reader produced, and nothing keys on a field's spelling. So the core's scene is a read model and an edit stays on each title's own records. Of the 293 lines in 91 files that name the crate at `9fdf0fee`, 59 name its streaming, transform, collision or portal modules and the rest GTA V's model.
- **Its neutral modules are not neutral as files.** The collision set is typed on GTA V's bounds through `crates/domain/or-world/src/collision/bound.rs`; its box type lives in `crates/domain/or-world/src/extents.rs` beside GTA V's maps; `crates/domain/or-world/src/transform.rs` holds the rotation convention of GTA V's entity records beside two neutral functions; and the one function of `crates/domain/or-maps/src/scene.rs` walks a project's GTA V sources.
- **The driver borrows and reads on the caller's thread.** `WorldForest` holds a borrowed index and source; GTA VI's streamer owns its index and a pool of readers.
- **Three edit vocabularies are bound to the project**: entities in `crates/domain/or-maps/src/operation.rs`, roads in `crates/domain/or-maps/src/roads/operation.rs` and collision in `crates/domain/or-maps/src/collision.rs`, each with named operations, inverses and digest guards. The session of `crates/domain/or-maps/src/operation/session.rs` is opened over a project folder, holds each edited source in memory and writes at a flush. A collision operation writes up to three versions of an area and its inverse is whole files; an entity removal renumbers a second file. `or-maps` depends on `or-project` and `or-source`, which the core cannot.
- **GTA V deletes with renumbering twice**: over value trees in `crates/domain/or-world/src/execute.rs` and over typed roots in `crates/domain/or-maps/src/delete.rs`.
- **The mount is GTA V's own, and its level is fixed at open.** `build_mounted_game_with` in `crates/title/or-title/src/vfs/game_mount.rs` takes an install, a crypto provider, a cutoff of DLC and mount options; `crates/domain/or-depends/src/world_index/mounted.rs` gives an archive no stamp where a level keeps folders out of the walk; the command line names its kept file by root and level; the fixtures mount unencrypted archives.

### Red Dead Redemption 2

An install is browsed as a folder of archives and nothing of its world is read. RPF8 archives open, any RSC8 resource opens as its bytes, and one format is declared, the texture dictionary of `crates/io/or-io/src/rdr2/texture.rs`, read only ([R56](Roadmap.md#r56-rdr2-the-second-container)). There is no mount, and a tagged archive lists its files by hash. Its map, type, model and bound files are declared by nothing, and its shader containers are not measured.

### The renderer

Three sets of game programs run today in three ways, and the GTA VI world runs none of GTA VI's. Nothing of GTA V Enhanced or RDR2 is run.

| What is drawn | Its surfaces | Its lighting and post | Where the programs come from |
|---|---|---|---|
| A GTA V drawable in Shaded, and particles | GTA V Legacy's own programs | GTA V Legacy's own | a pack the helper `or-shader-tool` bakes into the configuration folder |
| A GTA VI model in Shaded | GTA VI's own mesh and pixel programs, lowered in the process | OpenRage's own shaders | the install's effect containers, translated on a worker |
| The GTA VI world | OpenRage's own `WORLD_SHADER` of `crates/runtime/or-renderer/src/world/shader.rs`, in ten families the app maps each material onto | GTA V Legacy's lighting passes from that pack; OpenRage's own shaders for GTA VI's post, sky and water | no GTA VI program |

- **The host is in the app and typed on GTA VI.** Pages, the texture pool, culling, occlusion, budgets and readiness are the world files of `src/render/viewport/`; they name the viewer's model, material, texture, instance and chunk types and the occluder types of `or-io`. `src/render/viewport/world_budget.rs` and `src/render/viewport/world_pages.rs` import no title type.
- **The world shader has one vertex input and writes GTA V's four targets**, which GTA V Legacy's lighting then lights, so a family cannot share targets with a GTA VI surface program, which writes its own encoding.
- **The frame is one function.** `render_deferred` in `crates/runtime/or-renderer/src/viewport/game_frame.rs` draws a GTA V drawable and the GTA VI world, branching on whether the day carries GTA VI's post; a pass in the pack's manifest carries no render state.
- **A title-neutral draw list exists.** The renderer's `set_world` takes instances with no title type, and the overlay pass after the frame already draws lines and plain meshes over any world.
- **`or-renderer` names both games**: of the 29,535 lines under `crates/runtime/or-renderer/src/game_shaders/` at `9fdf0fee`, 2,971 non-test lines read as title-neutral, 9,232 as GTA V's and 5,478 as GTA VI's.
- **The translators are tools**: `or-shader-front` holds the three front ends and `or-cli` the GTA VI glue.

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
| The box scene of the Map screen | `crates/domain/or-maps/src/scene.rs` | `src/state/gta6_map.rs` | the scene types and the manipulator |
| A map body read for its nodes, its occluders and its lights | one read, of `crates/domain/or-world/src/map_data.rs` | three loops with three pools: the forest's, the occluders' and the lights' | nothing |

Inside GTA VI alone, the block that opens an archive for a worker and reads a body is written out in six files, and the bucket is stated in the app and again in the renderer.

## The shape

### The crates

| Crate | Layer | Owns | Depends on |
|---|---|---|---|
| `or-world`, the core | domain | The contract; the neutral read model (index rows, forest nodes, lights, occluders, a model and a texture as CPU data, the bucket, the selection); the stream policy; the driver, with the world thread and reader pool as a shell around it; the one kept-file mechanism; the queries; collision shapes; the neutral half of the transform; the manipulator and the scene types; the edit envelope; the key of a sentence; the conformance kit and the fake title | `or-job`, `or-title` for the game's id alone |
| or-world-gta5 | domain | First the contract over GTA V's crates as they stand; after R71n also its map and type model, index, mount source, bound-to-shape conversion, placement convention, find and placements. One crate for the Legacy and the Enhanced game | the core, `or-io`, `or-title`; `or-depends` until R71n |
| or-world-gta6 | domain | GTA VI's index, scan and mount binding, its map, interior, light and occluder readers, its material record, its streaming rules, its look, its edit operations, its render profile as data | the core, `or-io`, `or-title` |
| or-world-rdr2 | domain | What [R71k](#red-dead-redemption-2-r71k) finds it needs | the core, `or-io`, `or-title` |
| or-worlds, the registry | domain | One function that lists each title with its edit store, in the manner of `kinds` in `crates/domain/or-content/src/lib.rs` | every adapter, `or-maps` |
| `or-maps` | domain | GTA V's edit store, its project content kind, the builder of its box scene | the core, `or-project`, `or-source`; after R71n the GTA V adapter |
| `or-depends` | domain | After R71n: audio, the census, the export, the rules and the resolver | after R71n the GTA V adapter |
| `or-renderer`, the host | runtime | The frame host and the neutral fallback, typed on the core's model, and a module named for each title whose passes or fallback families it holds | the core, and each adapter whose module it holds |

Dependencies still run foundation, io, title, domain, runtime, app. `or-renderer` gains an edge to `or-world` and to each adapter it holds a module for, both down the chain ([ruling 2](#asked-of-the-owner)). The edge between the GTA V adapter and `or-depends` turns round at R71n, when the index and the driver become the adapter's. No adapter depends on another, on `or-renderer`, on a tool crate or on the app.

File reading stays in `or-io` for each game: GTA V's as declarations (D15), GTA VI's as readers by member hash, the three the app holds going down to `or-io` in R71b. Unification happens above the bytes.

## The contract

The outline below is what [R71a](#the-contract-the-kit-and-the-fake-title-r71a) builds; its names are proposals. The data contract is frozen at the end of R71e, when both titles draw a world and [R71m](#the-measures-that-decide-the-design-r71m) has RDR2's answers; the render profile is frozen after R71h, when a second title has one. After a freeze a change must leave every adapter's kit passing.

```rust
/// One value for each adapter crate.
pub trait WorldTitle: Sync {
    fn games(&self) -> &'static [Game];          // GTA V answers for two
    fn options(&self) -> &'static [OptionKind];  // what is chosen at open, as data
    fn open(&self, world: &WorldSpec, with: &Opening) -> Result<Arc<dyn WorldInstall>, WorldError>;
}
/// What is opened. All of it keys the session and the kept file.
pub struct WorldSpec {
    pub game: Game,
    pub install: PathBuf,
    pub options: Vec<(OptionId, OptionValue)>,   // GTA V: the level of DLC, story or multiplayer
    pub layers: Vec<Layer>,                      // read only, above the install, in order
}
pub struct Opening<'a> {
    pub config: &'a Path,     // kept files and translated programs
    pub keys: Keys<'a>,       // none, a snapshot, or the user's executable
    pub cancel: &'a Cancel,
    pub progress: &'a Progress,
}
/// One world, opened.
pub trait WorldInstall: Send + Sync {
    fn caps(&self) -> WorldCaps;
    fn policy(&self) -> StreamPolicy;
    fn extras(&self) -> Extras;
    fn profile(&self) -> Option<&dyn RenderProfile>;   // none: the fallback
    fn inputs(&self) -> Vec<Stamp>;                    // archives, and what else the index reads
    fn indexer(&self) -> &dyn WorldIndexer;
    fn maps(&self) -> &dyn MapReader;
    fn models(&self) -> &dyn ModelReader;
    fn look(&self) -> &dyn WorldLook;
}
pub trait WorldIndexer: Sync {
    fn rows(&self, archive: usize, out: &mut Section) -> Result<(), WorldError>;   // names unresolved
    fn whole(&self, archives: &[Section], out: &mut Section) -> Result<(), WorldError>;
    fn link(&self, archives: &[Section], whole: &Section) -> Result<Index, WorldError>; // names to ids
    fn placements(&self, map: MapId, out: &mut Vec<PlacementRow>) -> Result<(), WorldError>;
}
pub trait MapReader: Sync {
    fn read(&self, map: MapId, worker: &mut Worker) -> Result<Arc<dyn MapBody>, WorldError>;
}
/// A body is the title's own type, read once.
pub trait MapBody: Send + Sync {
    fn nodes(&self) -> &[Node];
    fn lights(&self) -> &[Light];          // point, spot, capsule, with fade distances
    fn occluders(&self) -> &Occluders;     // boxes and meshes
    fn interiors(&self) -> &[InteriorPlaced];
    fn lay(&self, drawn: &[NodeDraw], at: &Moment, out: &mut Instances); // once a map a selection
}
pub trait ModelReader: Sync {
    fn models(&self, asks: &[AssetKey], worker: &mut Worker, out: &mut Vec<(AssetKey, Model)>);
    fn textures(&self, asks: &[TextureAsk], worker: &mut Worker, out: &mut Vec<(TextureAsk, Texture)>);
}
pub trait WorldLook: Sync {
    fn switches(&self) -> Switches;        // changed while open
    fn frame(&self, at: &Moment, out: &mut dyn Feed) -> FrameFacts; // what the moment alone decides
}
/// Editing mirrors reading: a store is opened.
pub trait EditTitle: Sync {
    fn operations(&self) -> &[OperationKind];
    fn open(&self, place: &EditPlace) -> Result<Box<dyn EditSession>, EditError>;
}
pub trait EditSession: Send {
    fn guard(&self, at: &Address) -> Guard;   // a token unwritten, a digest written
    fn apply(&mut self, op: &Envelope) -> Result<Applied, EditError>; // the inverse, the maps changed
    fn body(&self, map: MapId) -> Option<Arc<dyn MapBody>>;           // asked before the reader
    fn flush(&mut self) -> Result<Vec<Written>, EditError>;
    fn base(&self, at: &Address) -> Option<Base>; // none: no comparison, put-back or merge
}
pub struct MapId(pub u32);                 // made by `link`, never stored
pub struct AssetKey(pub u64);              // the adapter's encoding of what draws an archetype
pub struct TextureAsk { pub scope: AssetKey, pub name: u64, pub pixels: u32 }
pub struct Model { pub lods: Vec<Lod>, pub title: Option<Arc<dyn Any + Send + Sync>> }
pub struct Instances {                     // columns, one entry an instance
    pub node: Vec<NodeId>, pub model: Vec<u32>, pub rows: Vec<[[f32; 4]; 3]>,
    pub sphere: Vec<[f32; 4]>, pub fade: Vec<f32>,   // below nought: fading out for its children
    pub words: Vec<u32>,                   // a block an instance, of the width the title declares
}
pub struct Envelope { pub kind: OperationId, pub at: Vec<(Address, Guard)>, pub body: Box<[u8]> }
```

Each part is set by code that exists:

| Part | What it is, and the code that sets it |
|---|---|
| The world opened | A stack, as [World_Editor_Plan.md](World_Editor_Plan.md#what-a-world-shows) defines one: an install up to a level, then read-only layers. GTA V's level changes the mount, the index and the kept file, so it is an option of the world, declared by the title as data, and never a switch. Two levels of one install are two sessions and two kept files |
| The opening | The GTA VI index asks the app for its configuration folder and key snapshot (`src/state/gta6_worldview/index_mount.rs`); GTA V's is handed a crypto provider. The kit opens a hand-built install with no key |
| The install | A reader needs state for each worker and the mounts share no type, so a game's one static value cannot also read. Capabilities, policy and profile are the install's because Legacy and Enhanced differ in them under one title |
| The indexer and the kept file | A head (the title, the version of the adapter's rules, the stamp of each input), a section of rows for each archive with names unresolved, one section the adapter derives across archives, and a link after every load. GTA V keeps the refresh that reads only what changed; GTA VI's derived tables and its `fresh_game.json` have a place |
| The body | Both titles' readers turn an entity into a `Node`. What a node draws is the title's: a time archetype by the hour, a list that becomes many instances or cards, a grass tint. So the core selects nodes and calls `lay` once a loaded map a selection, never a title's function a node. A body's lights and occluders reach the host with the selection |
| An instance | Fixed columns the host culls and picks by, and a block of the title's words: for GTA VI the ambient factors, the cast-only flag and the tint its shader reads today |
| The model reader | GTA VI finds a texture by one 64-bit key in a pack; GTA V names a model by a drawable and two optional dictionaries (`ModelFiles`, `Needed`) and a texture by a name inside a dictionary read whole. So an ask is a scope and a name, in batches the adapter groups by dictionary or pack |
| The model | The game's vertex streams with their layout, a small surface for the neutral fallback, its textures as asks, and the title's material record as an opaque value |
| The edit store | GTA V's is a session over a project folder that writes at a flush: after an apply the edited map is neither in the mount nor on disk, so the driver asks the session for a changed body first. An envelope addresses a set of files, each with its guard, and its inverse may restore bytes, as GTA V's three vocabularies need |
| Who keeps the steps | The caller, through a trait: the project's History for GTA V, as [World_Editor_Plan.md](World_Editor_Plan.md#one-vocabulary-of-edits) states; a document's history for GTA VI; memory for the fake. The core holds no stack |

The driver is a synchronous machine: wanted maps out, bodies in, a selection out. GTA V's command line drives it on the caller's thread over borrowed values; a screen wraps it in the world thread and the reader pool.

What stays out of the contract:

- **The mount and the address of a body.** An id is a row of a linked index; what it stands for is the adapter's.
- **The records an edit changes**, which are the title's, on the title's own file.
- **The title's day.** The core's frame facts are what the host and the neutral fallback need: the sun's direction, an ambient and a key light, an exposure and the clear colour. The rest reaches a title's programs through the feed, by name.
- **Far stand-ins, grass, water meshes, time archetypes, world-state selectors**: a title's extras, which its body lays out.
- **The project.** A finding of a core rule is the core's own small row, which the caller that has a project maps onto the row of `or_project::content::rule`.

## How a game differs

Shared code holds no test of which game it serves. A game differs at three levels, and a test fails when the core, the host or the app's world modules name one ([the developer's experience](#the-developers-experience)).

### Capabilities, as data

A capability says what the game has and what it can write back; a panel section, a tool or a verb whose capability is absent is hidden or greyed with the reason.

| Capability | GTA V | GTA VI | RDR2, expected |
|---|---|---|---|
| A height of the ground | yes, from the bounds `crates/domain/or-depends/src/world_collision.rs` streams | yes, from the install's height map | unmeasured |
| A collision query with hits and overlaps | yes: `crates/domain/or-world/src/collision/` | no | unmeasured |
| Placement rows and find | yes: `crates/domain/or-depends/src/find.rs` | rows for one archive, in the Map screen's store; no search | no |
| A weather gate on what streams | yes | none found | unmeasured |
| Chosen at open, in the world's options | the level of DLC, and story or multiplayer | nothing | unmeasured |
| Switched while open | the state groups at an hour | modes and script-managed groups (`set_enabled` of its streamer) | unmeasured |
| An edit store | yes, in a project | yes, over a document | no |
| A base to compare with, put back to and merge against | yes | no | no |
| Its own programs | Legacy after R71f, Enhanced after R71g | after R71h | no |

### Policies with defaults

A number is a field of the stream policy, a scale by category or level is a table, and a rule asked once a map is a function it holds; nothing of a policy is called once a node. The core writes the default and a game overrides only what it disagrees with. The cases the tree shows:

- **When a parent waits for its children.** A GTA V parent waits for exactly the children it states (`with_exact_children`); a GTA VI parent's count is raised to those loaded.
- **When a map streams in.** GTA V's rule is its box grown by `PRESTREAM` metres, taking the entities' box where the streaming one does not hold it (`streams_in`); GTA VI's is a sphere and a tier distance (`MapBox::wants`).
- **How a level fades.** `openrage-cli world select` fades every level over one distance; GTA VI reads its distances from the install.
- **Parents and categories.** A GTA V map has one parent and its nodes one category; a GTA VI map has a list of parents with a selector for each element, and its nodes a category read from load flags that scales by tables.

`crates/domain/or-world/src/streaming_rules.rs` and the constants of `crates/domain/or-world/src/interior_portals.rs` are GTA VI's build rules and move to its adapter; the portal walk itself is geometry and stays. The seven levels of `Level` stay the core's ladder, which both titles use.

### Extensions in slots

The core calls a slot and does not know who filled it:

- **Layers.** GTA V: those of [R70f](Roadmap.md#r70f-layers-of-what-the-game-hides). GTA VI: the lane graph, map lights, places with their outlines, the occluder tier.
- **Operations.** GTA V: its three vocabularies, the manifest, the first edit that imports. GTA VI: the selector of a parent map for each element, prop and grass lists.
- **Rules.** GTA V: `crates/domain/or-maps/src/rules.rs`. GTA VI: the link and child-count check the command line runs today.
- **Panel sections.** GTA VI: its modes and script-managed groups. GTA V: the level of DLC and the mode, which reopen the world.
- **Streamed extras.** GTA VI: far stand-ins as cards, grass, batch levels, time archetypes, the room and portal visibility.

A sentence a slot shows a person is a key the adapter returns and the app's catalogs hold. The key macro is the app's, and the language gate reads the app's source alone (`in_the_source` in `src/i18n/used.rs`), so R71a gives the core a key type with its macro and widens the gate's walk to the adapter crates.

RDR2's column is unmeasured throughout: R71m asks what the contract asks of a title before the data contract is frozen, and a need the contract cannot state is a change made then, with the kit as its check.

## Drawing

The owner chose option C: one shared frame host in which each game runs its own shader programs, with a plain fallback for what cannot be translated. The options rejected are in [the record](Project_Context.md#each-game-draws-its-world-with-its-own-shader-programs-in-one-shared-frame-host-2026-11-26).

### The host

The host is in `or-renderer` and owns the mechanics once: device pages and the texture pool, instancing, culling and occlusion, the streaming budget, render targets and the scheduling of passes, picking and read-back, and the editor's overlays. The overlays (the manipulator, the selection outline, the layers, collision wireframes) draw in a neutral pass after the game's frame, which the overlay pass already is, so they work for every title and in the fallback.

Most of it exists in the world files of `src/render/viewport/` and in `crates/runtime/or-renderer/src/world.rs`, and is moved, typed on the core's model; occlusion is typed on the occluders a body states, where `src/render/viewport/world_occlusion.rs` names GTA VI's today. What does not exist is pages that hold a game's own vertex layout, a pool bound by a game's sampler names, an id pass to pick by, and a scheduler of declared passes.

### The render profile

A profile is what a game states so that the host can run its programs. It is data and pure functions in the adapter, with no type of the graphics library:

- **Where its programs come from** in the install, the translator that lowers them, by name, and the key a lowered program is cached under.
- **What it needs of the device**, in the shape `device_needs` of `crates/runtime/or-renderer/src/game_shaders/meshlets.rs` has, so that a device that lacks it gets the fallback and the reason.
- **Its frame**: named targets with their formats, and a list of passes over them, each of a kind the host knows.
- **How a material binds**: which programs draw it in which pass, with the blend, depth and cull state the pass needs, or a refusal with its reason.
- **Its feed**: each frame, the values of its constants, by reflection name for GTA V and by binding and name hash for GTA VI, whose programs carry no names.
- **How a fallback surface joins its frame**: the semantics of each vertex layout, and by name the host's encoder that writes a fallback surface into its targets. A profile that names none takes the fallback for the whole frame.

GTA V states a vertex buffer laid out by the vertex stage's inputs, a material as variables and samplers by name, and four colour targets with a depth and stencil that holds material ids. GTA VI states the file's vertex data and meshlet records in storage buffers, a material as a record of words and a heap of textures by number, and up to four targets with a float depth and no stencil.

⚠ A list of passes cannot state today's GTA V frame whole, so "data, not code" is a target with a counted exception. The passes `render_deferred` schedules build a dome mesh, a dual-paraboloid sky map, cascade fits, light volumes as instanced spheres and room masks on the CPU. The host therefore has a small set of pass kinds, drawn from both frames that function draws; a profile's mesh for a pass is data the adapter makes; and a pass no kind states is a named pass in `or-renderer`, in the module named for its title, counted by a ledger a test holds. R71m counts the kinds and the named passes ([ruling 5](#asked-of-the-owner)).

The translators stay where they are. A domain crate may depend on neither `or-shader-front` nor `or-cli`, so the profile names its translator and the app hands the host one for each name, as `src/state/gta6_shader_translator.rs` and the helper process of `src/state/game_pack.rs` do today.

### The fallback

What cannot be translated, or what a device cannot build, is drawn in the fallback, and the panel lists what fell back and why. The neutral fallback is the host's own shader over a small surface a model states for each mesh: a colour map, a normal map where there is one, how its alpha is used and a tint. It needs no title module, and it is what the fake title, GTA V and a new game draw with; no per-shader port of GTA V's materials into it is built.

- **How it composes.** A fallback surface stands in a profile's frame only through the encoder that profile names; until R71m measures whether the two can share targets, a material that refuses puts its title's whole frame in the fallback ([ruling 13](#asked-of-the-owner)).
- **GTA VI's is richer and stays.** Its ten families are a hand port of what its programs do: the rejected option A, already built. The bucket is neutral and is stated once, in the core. The family is GTA VI's: the adapter's material record rides on the model as an opaque value, and the renderer's GTA VI module reads it with the mapping `src/render/viewport/world_material.rs` holds today. The families gain none from the day this plan lands. Until R71h they are GTA VI's whole frame, lit by a pack baked from a GTA V Legacy install, a borrow the architecture test lists by name ([ruling 4](#asked-of-the-owner)).
- **When a family goes.** Once every backend OpenRage supports draws every material that used it through its own program; on a backend that builds no GTA VI pass the families are the picture.
- **It is drawn on every gate.** A session has a fallback-only switch; the fake title has one material whose binding refuses, and each title's shots script takes one shot with the switch on.

### What the graphics library cannot run

- **Hull, domain and geometry stages.** wgpu has none. GTA V's tessellated and instanced technique variants bind them; the plain techniques build almost whole ([measured](Shader_Pipeline_Design.md#what-a-pass-needs)). A material whose only technique needs such a stage falls back.
- **GTA V's instancing.** The game's passes draw one drawable today, with one world matrix a scene. R71f measures the two ways to place an instance before it builds one.
- **GTA VI on Metal.** Its translated passes are built and drawn on Vulkan and on Direct3D with DXC; on Metal no device has built one ([GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md)). The owner's machine is a Mac, so R71m begins there.
- **GTA VI's lighting programs.** Only the surface pass of one model runs GTA VI's code; why its lighting is not run is one of R71m's measures.

### Where translated programs live

Translated programs come from the user's own install, are cached in the configuration folder and never enter the tree, the policy the keys follow. The cache becomes keyed by game and install, where today's GTA V pack is one folder for the machine. And the policy becomes a gate: the provenance lint's list of game content names the effect files and not a translated module, so R71f adds every cache's extensions to the lint and the ignore file.

## The app

The app has one world, three times over:

- **One session**, in `src/state/`, keyed by the world it opened (game, install, options, layers), held by a handle on the flat `AppState` (`the_shell_is_flat_rather_than_deep`). It owns the opened install, the camera, the switches, the world thread and the reader pools. No static holds a world. An install's tab and a project's tab are two sessions and draw two stacks at once, as [R70c](Roadmap.md#r70c-the-gta-v-world-streamed-and-drawn) asks.
- **One bridge**, in `src/render/viewport/`, that hands a session's selection and feed to the host and names no game.
- **One set of panels**, whose sections follow the capabilities and the slots. Of the World panel's sections today, Time and weather, Camera, Capture and Streaming are neutral; What streams becomes the switches as data, Mount the index's report as rows of a label and a value, and the blocked section's sentence the adapter's key.

Two kept shapes change: the World viewer's preferences, which store GTA VI's modes as a bitmask, are read once in their old form, and the kept indexes are rebuilt, as [R71b's third part](#the-gta-vi-world-behind-the-contract-r71b) says.

### The command line

A neutral verb takes a world as a screen does: an install, the game where the folder does not decide it, the title's options and its layers. A renamed verb keeps its old spelling for one release as a refusal that names the new one, since agents call verbs through the MCP server.

| Verbs | What becomes of them | In |
|---|---|---|
| `world index`, `at`, `ground`, `find`, `select` | neutral, for any registered title, by capability | R71d |
| `world info`, `check`, `extents`, `manifest`, `partition`, `create`, `new-map`, `states`, `prefab`, `to-xml`, `from-xml`, `archetypes` | GTA V's file verbs: unchanged, then spelled under the title's name | R71l |
| `world delete-entity`, `delete-map`; `maps apply`, `delete`, `edit-game`, `collision`, `roads`, `doors`; `rpf map-entity` of `crates/tools/or-cli/src/gta6_map_entity.rs` | edits: one apply verb that takes any title's operations through its store; a verb that only spells one operation refuses with its new spelling | R71i |
| `maps changes`, `merge` | by the capability of a base, which GTA V alone has | R71i |
| `maps list`, `validate`, `verify`, `keep-manifest` | GTA V's project verbs, unchanged | |

## The developer's experience

- **The conformance kit.** A macro in the core, in the manner of `declared_gates!` in `crates/io/or-io-verify/src/gates.rs`: an adapter's test file names its title and a fixture, and the macro writes the tests. The fixture is a stated scene (parents and children, a held map, a group at an hour, an interior, a list, a light, an occluder) in the adapter's own form. Among its rules: a reader that counts fails on a second read of a held map, and after an apply the selection shows the change with nothing flushed.
- **Two tiers.** One over rows and bodies, which every adapter reaches, and one over an install written to a scratch folder and opened through the contract with no key, which GTA V reaches through the fixtures of `crates/domain/or-depends/tests/`. For GTA VI the texture tests show a written resource is scanned; a map body with its registration documents is unchecked, and R71b measures it from that fixture.
- **The fake title.** A public module of the core, in no registry a person sees: a few maps, a cube, a flat texture, a two-pass profile in the shader language the host reads directly with one material that refuses, and a store that keeps its edits in memory.
- **Architecture tests**, beside the gates of `src/doc_counts.rs`. The core's source, the host's world modules outside those named for a title, and the app's world session, bridge and panels name no game; no adapter's manifest lists another adapter, the renderer, a tool or the app; no static holds a world. Each has a ledger of what still breaks it on the day it lands: the GTA V modules of the core and each file R71n splits, the GTA VI types of the bridge, the borrowed lighting pack. An entry that no longer breaks it fails the test.
- **Checks that run.** The recipe and hosted CI pass the self-check no viewport, so a row that must see a frame names `just self-check --viewport` in full and adds a test of `or-renderer` of the kind `crates/runtime/or-renderer/src/world/tests.rs` holds.
- **Gates keyed on a path move with the code.** A row that moves a suite re-keys the slow tier of `.config/nextest.toml` and the list of `crates/tools/xtask/src/slow_tier.rs` in the same commit, and checks no install or corpus suite of the moved crate is left in the default tier.
- **What adding a game costs.** Always: a profile and a mount in `or-title`, readers in `or-io`, one adapter crate with the kit's macro in its tests, one registry line, and its keys in the catalogs; a test holds that list, as `crates/title/or-title/tests/adding_a_title.rs` holds it for a profile. Where it runs its own programs: a translator among the tools with the app's hand-off, and a module of `or-renderer` for the passes no kind states. The fake title proves the path that needs neither.

## The order of work

A row is one programme part, lands in the parts its brief lists, and ends on a check that failed once before it passed. A row starts when the rows it depends on have merged.

| Row | Content | Depends on |
|---|---|---|
| R71a | The contract, the kit and the fake title | |
| R71m | The measures that decide the design | nothing; its questions are R71a's |
| R71d | The GTA V world behind the contract, over its crates as they stand; the registry; the neutral verbs | R71a |
| R71b | The GTA VI world behind the contract, in five parts | R71a; from part 2, R71d |
| R71c | One session, one host, panels by capability, in four parts | R71b |
| R71e | The GTA V world drawn in the fallback pass | R71c, R71d |
| R71n | The GTA V files moved into the adapter | R71d; the finished branches of R70 landed |
| R71f | The render profile, and the programs of GTA V Legacy | R71e; R71m's count of pass kinds |
| R71g | The programs of GTA V Enhanced | R71f; the Enhanced inventory of [R58f](Roadmap.md#r58f-shader-baking-off-windows) |
| R71h | The programs of GTA VI over a world | R71c; R71f for the pass kinds; R71m's three measures |
| R71i | One edit stack | R71b, R71d |
| R71j | The world tools built once | R71c, R71e, R71i |
| R71k | Red Dead Redemption 2 | R71m's answers; [R56](Roadmap.md#r56-rdr2-the-second-container) for the formats |
| R71l | Leftovers and the ledger | every row above |

- **R71m, R71d and R71b's first part run together** after R71a; R71i runs beside the drawing rows.
- **R71d lands before R71b's second part.** Both add a member to the workspace manifest and the lock file and a line to the registry; the second to land rebases those lines. So GTA VI's own loop is deleted, and its outputs rewritten, only after both adapters call the core's driver and GTA V's model names have gone through the reader.
- **R71a edits two files the branches of R70 change**, by a few lines: `crates/domain/or-world/src/streaming.rs` and the one constructor call in `crates/domain/or-depends/src/world_forest.rs`. It lands first and those branches rebase over it ([ruling 7](#asked-of-the-owner)). R71d edits no file of the three crates.

Rules for every row: no window is opened, and a screen is checked on `just shots` and `just self-check --viewport`; documents travel with the code; a moved test keeps its body; and a format's read path changed by a move runs that format's slow suite.

### The contract, the kit and the fake title (R71a)

Read `crates/domain/or-world/src/streaming.rs`, `crates/domain/or-depends/src/world_forest.rs`, `src/state/gta6_worldview/stream.rs` with its world thread, `crates/domain/or-depends/src/world_index/kept.rs`, `src/state/gta6_worldview/index_keep.rs`, `crates/title/or-title/src/vfs/game_mount.rs`, `src/state/gta6_worldview/texture.rs`, `crates/io/or-io-verify/src/gates.rs` and `crates/title/or-title/tests/adding_a_title.rs`.

- **Builds.** In `crates/domain/or-world/src/`: the contract and the neutral model, each part fixed against both titles' code as [its table](#the-contract) says; the stream policy, its scales as tables; the driver as a synchronous machine that asks an edit session for a body before the reader, reads a held map once and calls `lay` once a map, with the world thread and pool as a shell around it; the kept-file mechanism (head, sections, link, written beside and renamed, with a verdict of absent, current, changed, foreign or refused); `Pose` of `src/ui/panels/world_viewer.rs` without its two methods over the renderer's camera; the key type, its macro and the language gate's wider walk; the kit's macro and its first tier; the fake title; the architecture tests with their ledgers.
- **Deletes.** `with_exact_children`, once its one caller passes a policy; `streams_in` stays where it is and is what GTA V's policy holds.
- **Check.** The fake title streams and selects through the kit: a parent draws until its children are loaded and the children after. The kit run against a fake built to break each rule fails by that rule's name. A key a fake adapter returns that no catalog holds fails the language gate. A line naming a game added to a core module outside the ledger fails the architecture test. The GTA V forest's suites and the 25 tests of `src/state/gta6_worldview/stream_tests.rs` at `9fdf0fee` pass with no body changed.
- **Must not touch.** The GTA V modules of `or-world`, `or-maps`, and the viewer beyond the one call that passes a policy.
- **Not known.** Whether the machine serves GTA VI's chunks, swaps and bands without a second driver: R71b's fourth part answers by deleting its own.

### The measures that decide the design (R71m)

Read [GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md), `crates/runtime/or-renderer/src/viewport/game_frame.rs` and `crates/io/or-io/src/rdr2/`.

- **Builds.** Nothing that ships; each answer is recorded with its command and revision in the document that owns its topic.
  - *GTA VI's programs.* The share of the install's programs that translate and bind (`openrage-cli rpf scan-rdna-bind`); whether a Metal device on the owner's machine builds a pass (`or-shader-tool rdna-devices`); why the build's lighting programs are not run.
  - *The frame as a list.* `render_deferred` read as targets and passes for both frames it draws, the kinds counted and each pass no kind states named; whether a fallback surface and a game's program can write one set of targets.
  - *RDR2, by the contract's questions.* How its archives mount and in what order; how a map, a type and a model are named; whether its maps and types are resources or metadata; whether it has interiors, lists and a ladder of its own; the unit a model and a texture are read in; what its shader containers hold.
- **Check.** Every question has an answer or the reason it could not be had.
- **Must not touch.** Any code path a screen or a verb runs.

### The GTA V world behind the contract, and one kept file (R71d)

Read the header of `crates/domain/or-world/src/lib.rs`, `crates/domain/or-depends/src/world_index.rs` with its folder, `crates/domain/or-depends/src/world_forest.rs`, `crates/title/or-title/src/vfs/game_mount.rs` and the fixtures of `crates/domain/or-depends/tests/`.

- **Builds.** or-world-gta5 as a new crate over the core, `or-depends`, `or-io` and `or-title`. Its title: the options (the level by the mount's cutoff, story or multiplayer), the layers through the mount's options, the keys of the opening; the indexer over the mount; the map reader of `crates/domain/or-depends/src/world_forest/read.rs`, with the occluders and lights a map states; its policy, its state gate as switches, its interiors in the forest, its capabilities; and what names a model and its textures, as asset keys and asks grouped by dictionary. The registry crate, with its first line. The neutral verbs of [the command line](#the-command-line). Before a verb changes, the output of `world select` at fixed cameras over both installs is captured for R71e.
- **Check.** The kit passes for GTA V in both tiers, the second opening the hand-built install with no key. At fixed cameras over that install and both real ones, the adapter's selection through the core's driver is the selection `WorldForest` makes: the one check that holds the core's driver to a driver it did not write. Two worlds of one install at two levels share no index and no kept file. A second start opens no archive that did not change. The neutral verbs answer over the fake title in the default tier, and the suites of the verbs that exist pass with no body changed.
- **Must not touch.** Any file of `or-world`, `or-depends` or `or-maps`.
- **Not known.** Whether GTA V's rows can go through the core's kept file by the public sealing of its own; if not, GTA V keeps its file until R71n.

### The GTA VI world behind the contract (R71b)

Read [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md), `src/state/gta6_worldview/mod.rs`, `select_asking` in `src/state/gta6_worldview/stream.rs`, `src/state/gta6_worldview/model.rs` with `meshes_at` of `src/render/gta6_drawable.rs`, `src/self_check/shots/compare.rs`, and the shots steps of [Development_Workflow.md](../guides/Development_Workflow.md#screenshots).

It lands in five parts, each on its own check:

1. **The reference, made able to fail.** No World shots script is in the tree; a comparison whose reference is missing passes as a candidate, and it fails on a mean alone. Write the script: fixed cameras, hours and modes, `world settle`, `world converge`, a shot and a `world dump` at each. Make a missing reference a failure and bound the share of pixels past 16 levels beside the mean, both bounds set from two runs of the unchanged tree. Compare dumps over a stated list of fields, timings left out. A dump holds nothing of vertices, materials, lights, sky or water, so add an install-gated suite of the slow tier on the unchanged code: for fixed maps, models and textures, digests of each model's vertex and index bytes and material record, each texture's level bytes, the index's rows, and the look's frame values at fixed hours. References and digests live where [ruling 12](#asked-of-the-owner) says. The part ends when an altered reference and an altered digest each fail.
2. **The closed move.** By the closure of imports at `9fdf0fee`, 13 files can leave the app before the selecting half. Seven change in imports alone (2,051 lines, 34 tests): the reader pool, to the core; `src/state/gta6_worldview/index_types.rs`, `src/state/gta6_worldview/lights_lod.rs` and `src/state/gta6_worldview/lights_index.rs`, to `or-io`; the lights' law and variables and the material, to the adapter. Six follow after two cuts (1,888 lines, 20 tests): the index with its kept file, mount binding, scan and states, and the occluders. The cuts: the block of `src/state/gta6_worldview/index.rs` that implements a method of the app's `Viewer` (`land_and_water`) stays in the app, and the mount binding takes the configuration folder and the key snapshot from the opening. The adapter's registry line and the slow tier's keys land here.
3. **The kept file.** The index goes onto the core's mechanism: its tables derived across archives in the install-wide section, `fresh_game.json` an input the head stamps, ids made by the link. The first open deletes the old kept file of that install, reads `fresh_game.json` once from the old place and moves it, and shows the rebuild's progress; the rebuild's time on the build is recorded, and a later change of the rules' version says it costs every user that rebuild.
4. **The selecting half.** The two symbols it takes from the renderer move below it; then `src/state/gta6_worldview/stream.rs` with its five child modules, the interiors and the room walk move, the block of `src/state/gta6_worldview/stream_grass.rs` on `Viewer` staying in the app. The world thread is rewritten generic over a driver and enters the core. `select_asking` is split: the mechanics every title shares go to the core's driver, and GTA VI's rules (the held codes, time archetypes, batch levels that hide a parent's instances, cards that hand off to trees, the walk that marks an instance as casting only) stay in its body's `lay` and its policy. The occluders and the lights leave their own loops and pools for the one read of a body. `crates/domain/or-world/src/streaming_rules.rs` moves to the adapter but for the two functions the forest calls.
5. **The outputs.** The 18 other files. The model reader returns the core's model: the vertex decode of `src/render/gta6_drawable.rs` goes below the app with a neutral vertex, and `meshes_at` stays, calling down, with its skinning. The texture reader answers batched asks, and `src/state/gta6_texture_index.rs` goes with it. The sky, wind, water and terrain files return frame facts and feed the rest by name; their blocks on `Viewer` stay in the app.

- **Deletes.** `src/state/gta6_worldview/index_keep.rs`; the GTA VI streamer's own loop around the forest; the occluders' and the lights' loops and pools; the block that opens an archive for a worker, written once in place of six times.
- **Check.** After each part the shots, dumps and digests of part 1 match, with no candidate line. Of the 189 tests at `9fdf0fee` none is deleted, the 34 of part 2's seven files change in imports alone, and the pull request lists each test restated, the two texture tests over an archive among them. After part 3 a second start opens no archive and an edit of `fresh_game.json` rebuilds. After part 4 one caller of the forest's `select` is left outside tests, and the counting reader sees one read of a held map. The kit's first tier passes. The tests of `src/state/gta6_edit_tests.rs` and the Map screen's pass with no body changed.
- **Must not touch.** What the picture is: no family, pass or constant changes. What the Map screen and the model viewport do: the edits allowed there are the viewer's declaration in `src/state/gta6_world.rs` and the vertex decode part 5 moves. The singleton, which stays in the app as a shell over the crate until R71c.
- **Not known.** Whether two runs of the script agree on this machine; if not, the digests and dumps are the check. Whether `or-io` can write a map body with its registration documents for the kit's second tier.

### One session, one host, panels by capability (R71c)

Read `src/render/viewport/world.rs` and its neighbours, `crates/runtime/or-renderer/src/world.rs`, the five world viewer panel files, `src/self_check/shots/world.rs` and `src/ui/panels/status_bar.rs`.

It lands in four parts, each on the shots, dumps and digests of R71b:

1. **The session** of [the app](#the-app); `viewer()` gone from every call site and the shots steps addressed to a session; the five single-valued statics made fields of a session or of the opened install; the fallback-only switch, with its shot added to the script.
2. **The host.** The bridge's mechanics moved to `or-renderer` a module at a time, beginning with `src/render/viewport/world_budget.rs` and `src/render/viewport/world_pages.rs`; occlusion typed on the core's occluders; the selection's instances carrying their node; `src/render/viewport/world_material.rs` moved to the renderer's GTA VI module; the bridge reduced to handing a session to the host.
3. **The panels and the preferences**, by capability and slot, the old preferences read once.
4. **The fake title drawn**, by the self-check and by a test of `or-renderer` that fails when the frame paints nothing, the renderer's test packs made reachable outside its tests.

- **Deletes.** The static viewer and the five process-wide values; the app's bucket, stated once in the core.
- **Check.** Two sessions opened in one process share no exposure, radiance table or night map, in a test that fails on a static. `just self-check --viewport` draws the fake title's world, its refusing material in the fallback. The architecture test's ledgers for the bridge and the panels are empty. The French screens of the world hold at the smallest window.
- **Must not touch.** The frame: `render_deferred` is still the one frame, and the fallback's families are unchanged.
- **Not known.** Whether overlays drawn after GTA VI's temporal pass carry the frame's jitter; a shot with a line over a still world answers it.

After part 1 the other GTA VI sessions resume ([ruling 10](#asked-of-the-owner)) on every file but those the later parts move: the world files of `src/render/viewport/`, the world viewer's panels and preferences, and `src/self_check/shots/world.rs`.

### The GTA V world drawn in the fallback pass (R71e)

Read `crates/runtime/or-renderer/src/world.rs`, `crates/runtime/or-renderer/src/game_shaders/drawable.rs`, and [the Gen8 and Gen9 section](../reference/Crate_Reference.md#gen8-and-gen9-enhanced-is-a-separate-target) of the crate reference.

- **Builds.** The decode behind the model names R71d answers: a drawable, a drawable dictionary's entry and a fragment to the core's model, for both targets, each by its file's own RSC7 version; a texture dictionary read once for the asks it holds; a material's fallback surface by its sampler names. The World room for a GTA V install, read only.
- **Check.** A synthetic world of parent and child levels draws the level the rules select at each distance, in real frames with no window. Over each install, at R71d's cameras, the instances drawn are those of the file R71d captured, and every model not drawn has a named reason. The frame over the city is measured on the owner's machine and its budget recorded. The data contract is frozen here, citing R71m's answers for RDR2.
- **Must not touch.** The Shaded viewport of one drawable; GTA VI's picture.
- **Not known.** The frame's cost and the memory of the city's textures, which this row measures before R71f builds on it.

### The GTA V files moved into the adapter (R71n)

Read the manifests of `or-depends` and `or-maps`, and `crates/domain/or-depends/src/resolve.rs`.

- **Builds.** One mechanical commit on `main` between waves, with no other lane open on the three crates. The GTA V modules of `or-world` move with `git mv`. The splits: `crates/domain/or-world/src/collision/bound.rs` and the half of the collision set that faces a bound go to the adapter, and the box type is lifted out of `crates/domain/or-world/src/extents.rs` into the core; the placement convention of `crates/domain/or-world/src/transform.rs` goes, and `scale_of` and `placed_corners` stay; the scene's types go to the core and its builder stays in `or-maps`. The world half of `or-depends` moves with the catalogue, the tables, the contents, the two types of the resolver its index names and `crates/domain/or-depends/src/tests_support.rs`, so that the resolver left behind imports the adapter and no ring forms; the script forms of `crates/domain/or-maps/src/script_text.rs` and the filename rule of an area's bounds go with it, and the place parser, the manipulator and the pose go to the core, which ends the edge from `or-depends` to `or-maps`. `WorldForest` becomes a thin wrapper over the core's driver with its public surface as it is, and GTA V's index is kept by the core's mechanism.
- **Deletes.** `crates/domain/or-depends/src/world_index/kept.rs`; GTA V's own loop inside `WorldForest`; the include by path of `or-depends`' fixtures into `or-cli`.
- **Check.** The 17 tests of `crates/domain/or-depends/tests/the_forest_of_a_mount.rs`, `crates/domain/or-depends/tests/the_world_index_of_a_real_install.rs` and the world suites of `or-cli` pass with no body changed, counted before and after with none deleted. `cargo tree` shows no ring. One world kept-file format is left in the tree. The slow tier holds every install suite it held, under its new key.
- **Must not touch.** What a GTA V file reads or writes as.
- **Not known.** Which of the two GTA V deletions with renumbering survives; R71l settles it.

### The render profile and the programs of GTA V Legacy (R71f)

Read [Shader_Pipeline_Design.md](Shader_Pipeline_Design.md), `crates/runtime/or-renderer/src/viewport/game_frame.rs`, `crates/runtime/or-renderer/src/game_shaders/mesh.rs` with its scene, and `src/state/game_pack.rs`.

It lands in three parts:

1. **The frame as a list.** Restate `render_deferred` as targets and passes of the kinds R71m counted. Give a pass of the pack's manifest its render state. Key the pack by game and install. Add every cache's extensions to the provenance lint and the ignore file.
2. **An instance of a game's program.** Measure a city view drawn with a draw for each instance against a lowering that reads the transform by instance, and build the one the measure favours. Lay the pages out by the vertex stage's inputs, their semantics stated so that a fallback surface binds them. Bind the streaming pool's textures by sampler name.
3. **The world through the profile**, with GTA V's day from the install's timecycle as its feed, which is the feed [R70d](Roadmap.md#r70d-the-day-sky-water-and-shadows-of-the-world) asks for.

- **Deletes.** `render_deferred`, once the list draws its frame; the one pack folder for the machine.
- **Check.** The frame drawn from the list equals the frame `render_deferred` drew, for a drawable and for the GTA VI world, before that function is deleted. A drawable in Shaded and the same drawable placed alone in the world at the same hour and weather agree within a recorded tolerance. With no pack, or with the fallback-only switch, the world draws in the fallback and says so, and the panel lists each material that fell back with its reason. A translated module staged in a commit fails the hook.
- **Must not touch.** The helper's protocol; no C++ enters the editor.
- **Not known.** The cost of either way to place an instance, the first measure of part 2.

### The programs of GTA V Enhanced (R71g)

Read [Shader_Definition_Database.md](../features/Shader_Definition_Database.md) for the two containers and `crates/tools/or-shader-front/src/dxil/`.

- **Builds.** A bake that drives the Rust front end over Enhanced's containers into a pack of the same manifest, and Enhanced's profile over it.
- **Check.** The inventory R58f leaves open is recorded. The share of Enhanced's programs that lower and that each backend's writer takes is measured and recorded. Over an Enhanced install the world draws through its own programs, and what does not is listed with its reason.
- **Must not touch.** Legacy's pack and bake, and the helper's protocol.
- **Not known.** Everything: no bake has translated an Enhanced program, and until this row closes an Enhanced install draws in the fallback.

### The programs of GTA VI over a world (R71h)

Read R71m's three measures, `crates/runtime/or-renderer/src/game_shaders/meshlets.rs`, `src/render/gta6_meshlets.rs` and `src/state/gta6_shaders.rs`.

- **Builds.** GTA VI's profile: its targets and passes; its surface pass over the host's pages as storage buffers; a transform for each instance, which nothing feeds today; the bindings only the game fills, named; and the encoder its fallback surfaces write its targets with, or the whole frame in the fallback where R71m found no mix. Then its lighting, by its own programs where the measures allow and by named passes where they do not.
- **Deletes.** A family of the fallback's shader once every supported backend draws every material that used it; the renderer's mapping of a material onto a family with the last of them.
- **Check.** A model drawn alone in Shaded and placed alone in the world agree within a recorded tolerance. A map's materials draw through their own programs, and each that falls back is listed with its reason. The script's fallback-only shot still matches. The borrowed GTA V pack is gone from the ledger, or [ruling 4](#asked-of-the-owner) says it stays. The render profile is frozen here.
- **Must not touch.** A family any backend still draws with.
- **Not known.** Whether this is possible on the owner's machine at all, which R71m's Metal measure answers first.

### One edit stack (R71i)

Read `crates/domain/or-maps/src/operation.rs` with its session, the two other vocabularies named in [GTA V](#gta-v), `crates/domain/or-maps/src/changes.rs`, `src/state/workbench/maps.rs`, and the three GTA VI entity edit files of `src/state/` with `crates/io/or-io/src/gta6/map_grow.rs`.

- **Builds.** In the core: the envelope, stated against GTA V's three vocabularies (a set of addresses, a guard for each, an inverse that may restore bytes); its ordering rules; the trait of who keeps the steps; and the skeleton of comparison, put-back and merge over a trait that gives an entity's identity. GTA V's store stays `or-maps`: its session behind the edit session's trait, answering a changed body to the driver, its steps kept by the project's History. GTA VI's operations are new, in its adapter, as pure functions over the sections of a body: place, move, turn, scale, retype, add and remove across linked bodies, each with its inverse, guarded by a digest of that body. The app's document stays the owner of GTA VI's bytes and the keeper of its steps, and lends the body to those functions ([ruling 14](#asked-of-the-owner)). One apply verb on the command line for any title's operations.
- **Deletes.** The three GTA VI entity edit files of `src/state/` (2,162 lines at `9fdf0fee`), `src/state/gta6_map_extents.rs`, the second history of the Map screen, and the verbs [the command line](#the-command-line) turns into refusals.
- **Check.** The kit's edit tests pass for both titles and the fake: every operation followed by its inverse leaves the store byte for byte, a stale guard refuses, an operation with no inverse fails a test, and after an apply the driver's selection shows the change with nothing flushed. The apply verb runs the same tests with no window. The tests of `or-maps` pass with no body changed. The GTA VI edit tests are restated with no `AppState`, none deleted.
- **Must not touch.** GTA VI's navmesh and collision edits, which stay document steps for their cost ([ruling 6](#asked-of-the-owner)).
- **Not known.** Whether a scanned body can serve as GTA VI's base for a comparison; until it does, its capability says no.

### The world tools built once (R71j)

Read [World_Editor_UX_Design.md](World_Editor_UX_Design.md), `crates/domain/or-maps/src/gizmo.rs`, `src/render/viewport/pick.rs` and `src/ui/commands/target.rs`.

- **Builds.** The surface the screen rows of R70 stand on, for every title: the pick by an id pass, the selection outline and the manipulator in the overlay pass, the registry of layers and of panel sections, a command's capability in the command table, and placement rows as a capability. GTA VI's placement rows replace the Map screen's world store and its kept file.
- **Deletes.** `src/state/gta6_world.rs` and `src/state/gta6_world_fill.rs`.
- **Check.** On the fake title a click returns the node under the pixel, a drag applies one operation to its edit session, the next selection draws the node moved through the session's body, and Undo takes it back, in real frames with no window. A tool whose capability a title lacks is greyed and says why. The plan overlay of a GTA VI archive draws the same shots as before.
- **Must not touch.** What each tool does for a mapper, which the rows of R70 own.

### Red Dead Redemption 2 (R71k)

Read R71m's answers, [R56](Roadmap.md#r56-rdr2-the-second-container) and `crates/io/or-io/src/rdr2/`.

It lands in three parts: a mount in `or-title`; the formats in `or-io`, which R56 gates; and the adapter, view only, in the fallback pass.

- **Check.** The adapter passes the kit's first tier, and an install draws with every model not drawn named.
- **Must not touch.** GTA V's mount: the function that mounts GTA V is not widened to serve RDR2.
- **Not known.** Every figure. R56 warns that each estimate made there before a format was measured was low, and this row gives none.

### Leftovers and the ledger (R71l)

- **Builds.** Nothing new. It settles the two GTA V deletions, spells GTA V's file verbs under the title's name, moves the conversions the renderer still holds for a title into that title's feed where a profile now serves them, and corrects every document that states the old shape.
- **Deletes.** Whichever of the two GTA V deletions loses, and every ledger entry no ruling keeps.
- **Check.** Every ledger of the architecture tests is empty or holds only entries a ruling names, and an entry added to one fails. The lines each duplicate of [what is written twice](#what-is-written-twice) lost are published as a table measured with `git diff --shortstat` between `9fdf0fee` and the closing revision.

## What it changes for R70

What is finished of [R70](Roadmap.md#r70-the-gta-v-world-editor) lands as it is, and each of its rows in the Roadmap says what this programme changes for it. In short:

- **The headless parts** (the index of R70b, the find of R70e, the collision of R70g) keep their checks, are reached through the adapter in R71d and move crate in R71n. The index is opened as a world with its level; the height of the ground and the collision query are two capabilities.
- **The world drawn** (R70c, R70d): the seam, the viewer for each tab and the feed are R71c, R71d, R71e and R71f; R70c keeps the room, the tab, the bar of the stack, the camera's inputs and the presets. R70r's server is a layer of the world opened.
- **The edits** (R70h and the tools of R70j to R70p): the vocabularies stay GTA V's store and a world step stays a step of the project's History; the envelope and its ordering rules are the core's in R71i, and the tools draw in the overlay pass.
- **The screens** (R70e, R70f, R70i, R70v, R70y, R70z) are built once, on the session and the tools of R71j, for every title that has the capability.

Three statements of [World_Editor_Plan.md](World_Editor_Plan.md) are replaced: that the seam between the titles is declared in the app beside the GTA VI viewer, that a GTA VI world keeps its own World panel, and its ruling 9, which the owner's choice of option C answers.

## Risks

Each is answered in the row named:

- **The move breaks the picture**: R71b's shots, dumps and digests, compared part by part by a comparison shown to fail.
- **The contract is shaped by one title**: GTA V answers it in R71d, before GTA VI's loop is deleted or the host is typed on the model, and R71m asks RDR2's questions before the freeze.
- **A third driver is written and the two stay**: R71b's fourth part and R71n each end with their own loop deleted.
- **The local branches of R70 conflict with the move.** On 2026-11-27, of the 41 local branches of R70, 40 are ahead of `origin/main` at `9fdf0fee`; 33 of them change the three crates and four change the two files R71a edits (`git diff --name-only` from each merge base). Which are unlanded work is the owner's list ([ruling 7](#asked-of-the-owner)); R71d moves no file and R71n waits for that list.
- **A frame cannot be stated as a list, the city costs more than the frame has, or GTA VI's programs do not build on the owner's machine**: R71m and R71e measure before R71f and R71h build.
- **Nothing draws the neutral fallback once profiles work**: the fallback-only switch.
- **Game content enters the tree as a translated module or a reference shot**: R71f's lint; references live outside the tree.
- **A capability becomes a test of the game by another name**: a branch on a capability that changes what a shared rule does is a policy and is written as one.

## Rulings

The three made on 2026-11-26 are recorded under the links of the status line.

### Asked of the owner

Each has a default a row takes until a sentence reverses it.

1. **A registry crate lists the titles**, where the agent guide says there are no facade crates. Default: yes; it re-exports nothing, as `or-content` for the kinds of content.
2. **`or-renderer` depends on `or-world`, and on each adapter whose module it holds.** The plan of 2026-11-25 said it does not depend on the first. The alternative to the second is an adapter that lowers its material to a record of words the core defines. Default: both edges; they point down the chain, and the mapping that exists moves as a file.
3. **The translators stay tools and the app hands them to the host**, where the alternative moves the Rust front ends into the io layer. Default: they stay.
4. **The GTA VI world keeps GTA V Legacy's lighting pack until R71h**, and its families on a backend that builds none of its passes. Default: both stay, in the ledger.
5. **No line that touches the device enters an adapter**: a game is one adapter crate and, where it names passes, one module of the renderer. Default: yes; the owner is asked again should R71m's count refute it.
6. **GTA VI's navmesh and collision edits stay document steps.** GTA V's collision and road edits are operations under the same envelope, so the reason is cost, not shape. Default: they stay out of this programme.
7. **R71a lands before the branches of R70 that change its two files, and R71n waits for the finished ones.** Which are finished is the owner's to say. Default: as stated; R71d, R71b and R71c wait for none.
8. **GTA VI gains no comparison, put-back or merge in this programme.** Default: yes.
9. **RDR2 is view first and edit later.** Default: yes.
10. **The other GTA VI sessions resume when R71c's first part merges**, on the files its later parts do not move. The owner paused them "meanwhile" and did not say until when. Default: as stated.
11. **The core keeps the name `or-world`**, as agreed, so R71n rewrites the imports of GTA V's model: 234 of the 293 lines that name the crate. The alternative is a new crate for the core with `or-world` left as GTA V's model until R71l, which rewrites the other 59 and waits for no branch. Default: the name agreed.
12. **Where reference shots and digests are kept.** A frame of a game cannot be committed and a lane writes nowhere outside itself. Default: until the owner names a store, a missing reference is made again in scratch from the revision before the part, by the same script.
13. **A material that refuses puts its title's whole frame in the fallback**, until R71m shows a fallback surface and a game's program can share targets. Default: the whole frame.
14. **The app's document owns GTA VI's edited bytes**, and the adapter's operations are pure functions it lends a body to. The alternative is a store in the adapter with the document as its view. Default: the document.

## Sources

- The three readings of the tree at `9fdf0fee`, of 2026-11-26, and the two reviews of this plan against the code, of 2026-11-27.
- [World_Editor_Plan.md](World_Editor_Plan.md), [Shader_Pipeline_Design.md](Shader_Pipeline_Design.md), [Shader_Definition_Database.md](../features/Shader_Definition_Database.md), [GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md), [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) and [the layers](../reference/Crate_Reference.md#layers) of the crate reference.
