# Unified world plan

How the world viewer and the world editor become one title-neutral core with one adapter crate for each game, what the tree holds today, the contract between the two, how a world is drawn, and the order and the briefs of the rows that build it.

Status: the plan of 2026-11-26, written on the owner's decisions of that day ([the shape](Project_Context.md#the-world-viewer-and-editor-become-one-core-and-an-adapter-crate-for-each-game-2026-11-26), [who lifts the GTA VI viewer](Project_Context.md#this-programme-lifts-the-gta-vi-viewer-out-of-the-app-and-the-other-gta-vi-sessions-pause-2026-11-26), [the drawing](Project_Context.md#each-game-draws-its-world-with-its-own-shader-programs-in-one-shared-frame-host-2026-11-26)), mended after its review of 2026-11-27, and again that day for what its first two rows, R71a and R71m, built and measured; [R71](Roadmap.md#r71-one-world-for-every-title) says where each row stands.

## The shape in brief

**One core, `or-world`, that names no game, and one adapter crate for each of GTA V, GTA VI and Red Dead Redemption 2**, with one world session in the app, one frame host in the renderer, and each game's own shader programs run inside that host. Four facts of the tree shape the rows:

1. **A world is a stack whose level is fixed when it opens**, so the contract opens a world, never a folder, and an edit store is opened the same way.
2. **Lifting the GTA VI viewer is not a pure move, and GTA V answers the contract with no file moved**; its move is a later, mechanical row.
3. **The GTA VI world runs none of GTA VI's programs, and nothing draws the GTA V world.** The first is kept as GTA VI's fallback; the second is new work.

## What it rests on

- **The tree at `9fdf0fee`**, read on 2026-11-26: every line count here is `wc -l` at that revision and every test count its `#[test]` attributes.
- **The contract as built and the measures as taken**: [the contract](#the-contract) is read from the code, and a measure's figures are in the document that owns its topic.
- **The owner's three decisions of 2026-11-26**, and [World_Editor_Plan.md](World_Editor_Plan.md) ([what it changes for R70](#what-it-changes-for-r70)).

## What the tree holds today

Read at `9fdf0fee`, before any row was built; R71a added the core's modules beside GTA V's model and moved nothing.

### GTA VI

The app holds two GTA VI worlds, each a process-wide singleton with its own index and kept file: the World screen, which streams and draws an install, and the Map screen, which edits a scanned map body as a document.

At `9fdf0fee` the World screen's viewer is `src/state/gta6_worldview/` (55 files, 18,010 lines, 189 tests); the bridge that draws it, which is the host today, is the world files of `src/render/viewport/` (13 files, 4,651 lines); the Map screen's world store is `src/state/gta6_world.rs` with `src/state/gta6_world_fill.rs`; and map edits are steps of a document's undo, in `src/state/gta6_map.rs` and its neighbours.

- **The viewer is one static.** `viewer()` in `src/state/gta6_worldview/mod.rs` locks a `LazyLock<Mutex<Viewer>>`, and at `9fdf0fee` 23 call sites reach it, 10 of them in `src/self_check/shots/world.rs`. Five of the folder's statics are wrong for two sessions: in `src/state/gta6_worldview/sky.rs` the exposure, the sky radiance table with its worker's slot and the slot of the moment's worker, and in `src/state/gta6_worldview/sky_night.rs` the night maps of the install taken last.
- **The map index and the stream path have no test over an archive**; two tests of `src/state/gta6_worldview/texture_tests.rs` do write a resource with `write_rpf9_resource`, scan it in a scratch install and read a texture back.
- **The selection carries no identity**: an instance names no node, map or entity, and nothing picks, selects or moves in the World screen.
- **Three readers of a body by member hash live in the app**: `src/state/gta6_worldview/index_types.rs`, `src/state/gta6_worldview/lights_lod.rs` and `src/state/gta6_worldview/lights_index.rs`.
- **The index is whole-install tables**, several derived across archives, which `src/state/gta6_worldview/index_keep.rs` writes whole under a head that also holds a word of `fresh_game.json`, a file a person writes beside the index.

### GTA V

GTA V's world is headless: `openrage-cli world` reaches it and no screen draws it.

At `9fdf0fee` its model is the modules of `crates/domain/or-world/src/` outside the core's (12,996 lines); the index of an install with its kept file and mount source is `crates/domain/or-depends/src/world_index.rs` and its folder; the streaming driver and its map reader are `crates/domain/or-depends/src/world_forest.rs` and its folder; and the edit vocabularies with their session are `crates/domain/or-maps/src/` (22,178 lines).

- **`or-world` is GTA V's model with a core beside it.** Its header states two invariants the core respects: nothing there copies the tree a reader produced, and nothing keys on a field's spelling.
- **Three edit vocabularies are bound to the project**: entities in `crates/domain/or-maps/src/operation.rs`, roads in `crates/domain/or-maps/src/roads/operation.rs` and collision in `crates/domain/or-maps/src/collision.rs`, each with named operations, inverses and digest guards. The session of `crates/domain/or-maps/src/operation/session.rs` is opened over a project folder and writes at a flush. A collision operation's inverse is whole files, and an entity removal renumbers a second file. `or-maps` depends on `or-project` and `or-source`, which the core cannot.
- **The mount is GTA V's own, and its level is fixed at open**: `build_mounted_game_with` in `crates/title/or-title/src/vfs/game_mount.rs` takes an install, a crypto provider, a cutoff of DLC and mount options, and the fixtures mount unencrypted archives.

### Red Dead Redemption 2

An install is browsed as a folder of archives and nothing of its world is read. RPF8 archives open, any RSC8 resource opens as its bytes, and one format is declared, the texture dictionary of `crates/io/or-io/src/rdr2/texture.rs`, read only ([R56](Roadmap.md#r56-rdr2-the-second-container)). There is no mount, and nothing orders its archives. [R71m measured the rest](../reference/Crate_Reference.md#what-the-measures-found) on the owner's install:

- **Archives and names**, on 2026-11-27. 89 of 89 top-level archives and 4,871 of the 4,874 inside them open. Of their 488,718 files 353 carry a name; every other row is a 32-bit hash, by which a body names its models, its texture dictionaries and its parent maps.
- **Maps and type files are GTA V's by name** (2026-11-27): paged metadata that names its types and members by the hashes of GTA V's own names, all 34 looked for of a map and all 25 of a type file. They sit under an RSC8 head, which the metadata reader does not take, so nothing reads them; the measure finds a name's hash in a body and decodes nothing.
- **Models, fragments and bounds** have no reader. Of the four shader containers two hold Direct3D bytecode and two SPIR-V.

### The renderer

Three sets of game programs run today in three ways, and the GTA VI world runs none of GTA VI's. Nothing of GTA V Enhanced or RDR2 is run.

| What is drawn | Its surfaces | Its lighting and post | Where the programs come from |
|---|---|---|---|
| A GTA V drawable in Shaded, and particles | GTA V Legacy's own programs | GTA V Legacy's own | a pack the helper `or-shader-tool` bakes into the configuration folder |
| A GTA VI model in Shaded | GTA VI's own mesh and pixel programs, lowered in the process | OpenRage's own shaders | the install's effect containers, translated on a worker |
| The GTA VI world | OpenRage's own `WORLD_SHADER` of `crates/runtime/or-renderer/src/world/shader.rs`, in ten families the app maps each material onto | GTA V Legacy's programs from that pack for the shadow's receive, the occlusion, the lights, the sky, the fog volumes, the distant lights, the coronas and the resolve; OpenRage's own shaders for GTA VI's history, grade, bloom and sharpening | no GTA VI program |

- **The world shader has one vertex input and writes GTA V's four targets**, which GTA V Legacy's lighting then lights.
- **The frame is one function.** `render_deferred` in `crates/runtime/or-renderer/src/viewport/game_frame.rs` draws a GTA V drawable and the GTA VI world, branching on whether the day carries GTA VI's post, in 29 recorded calls whose order `crates/runtime/or-renderer/tests/the_frame_as_targets_and_passes.rs` holds; a pass in the pack's manifest carries no render state.
- **A title-neutral draw list exists.** The renderer's `set_world` takes instances with no title type, and the overlay pass after the frame already draws lines and plain meshes over any world.

### What is written twice

Each row is a duplicate the programme ends, at `9fdf0fee`:

| What | GTA V | GTA VI |
|---|---|---|
| The index of an install | `crates/domain/or-depends/src/world_index.rs` | `src/state/gta6_worldview/index.rs` and five neighbours |
| A kept file of rows, checked against the archives' size and time | `crates/domain/or-depends/src/world_index/kept.rs` | `src/state/gta6_worldview/index_keep.rs`, and a second in `src/state/gta6_world_fill.rs` |
| The driver around the forest, which both share (`Forest` of `crates/domain/or-world/src/streaming.rs`): wanted maps, read, insert, select, instances | `crates/domain/or-depends/src/world_forest.rs` | `src/state/gta6_worldview/stream.rs` and its neighbours |
| What is switched on at an hour | `crates/domain/or-world/src/state.rs` | `src/state/gta6_worldview/index_states.rs` |
| A map's extents grown to hold an entity | `crates/domain/or-maps/src/edit.rs`, `crates/domain/or-world/src/extents.rs` | `src/state/gta6_map_extents.rs` |
| Removing an entity and repairing links across maps | `crates/domain/or-world/src/execute.rs`, `crates/domain/or-maps/src/delete.rs` | `crates/io/or-io/src/gta6/map_grow.rs`, `src/state/gta6_map_entity.rs` |
| The undo of the Map screen | `src/state/workbench/maps.rs` | the document's own steps |
| The box scene of the Map screen | `crates/domain/or-maps/src/scene.rs` | `src/state/gta6_map.rs` |
| A map body read for its nodes, its occluders and its lights | one read, of `crates/domain/or-world/src/map_data.rs` | three loops with three pools: the forest's, the occluders' and the lights' |

## The shape

### The crates

| Crate | Layer | Owns | Depends on |
|---|---|---|---|
| `or-world`, the core | domain | The contract, the read model, the stream policy, the driver with the world thread and reader pool around it, the kept file, the queries, collision shapes, the neutral half of the transform, the manipulator and the scene types, the edit envelope, the key of a sentence, the kit and the fake title | `or-job` alone: a game is the core's own id |
| or-world-gta5 | domain | First the contract over GTA V's crates as they stand; after R71n also its model, index, mount source, find and placements. One crate for the Legacy and the Enhanced game | the core, `or-io`, `or-title`; `or-depends` until R71n |
| or-world-gta6 | domain | GTA VI's index, scan and mount binding, its readers, its material record, its streaming rules, its look, its edit operations, its render profile as data | the core, `or-io`, `or-title` |
| or-worlds, the registry | domain | One function that lists each title with its edit store and maps a registered game onto the core's id, in the manner of `kinds` in `crates/domain/or-content/src/lib.rs` | every adapter, `or-maps` |
| `or-renderer`, the host | runtime | The frame host and the neutral fallback, typed on the core's model, and a module named for each title whose passes or fallback families it holds | the core, and each adapter whose module it holds |

`or-renderer` gains an edge to `or-world` and to each adapter it holds a module for, both down the chain ([ruling 2](#asked-of-the-owner)). `or-maps` stays GTA V's edit store, over `or-project` and `or-source`; the edge between the GTA V adapter and `or-depends` turns round at R71n, which leaves that crate audio, the census, the export, the rules and the resolver. No adapter depends on another, on `or-renderer`, on a tool crate or on the app. File reading stays in `or-io`: GTA V's as declarations (D15), GTA VI's as readers by member hash.

## The contract

The outline below is the contract as [R71a](#the-contract-the-kit-and-the-fake-title-r71a) built it, in `crates/domain/or-world/src/contract.rs` and `crates/domain/or-world/src/model.rs`; where the two differ the code is right and this outline is mended. The data contract is frozen at the end of R71e, when both titles draw a world; the render profile, which is not in it yet, is frozen after R71h. After a freeze a change must leave every adapter's kit passing.

```rust
// contract.rs
pub struct GameId(pub &'static str);         // not or-title's Game
pub trait WorldTitle: Sync {
    fn id(&self) -> &'static str;            // in a kept file's head and name
    fn games(&self) -> &'static [GameId];    // GTA V answers for two
    fn options(&self) -> Vec<OptionKind>;
    fn open(&self, world: &WorldSpec, with: &Opening<'_>) -> Result<Arc<dyn WorldInstall>, WorldError>;
}
pub struct WorldSpec { pub game: GameId, pub install: PathBuf,
    pub options: Vec<(OptionId, OptionValue)>, pub layers: Vec<Layer> }   // identity() keys the session and the kept file
pub trait WorldInstall: Send + Sync {
    fn caps(&self) -> WorldCaps;
    fn policy(&self) -> StreamPolicy;
    fn extras(&self) -> Extras;
    fn inputs(&self) -> Inputs;              // archives and others: Input { name, stamp: Option<Stamp> }
    fn indexer(&self) -> &dyn WorldIndexer;
    fn maps(&self) -> &dyn MapReader;
    fn models(&self) -> &dyn ModelReader;
    fn look(&self) -> &dyn WorldLook;
}
pub trait WorldIndexer: Sync {
    fn rules(&self) -> u32;
    fn rows(&self, archive: usize, out: &mut Section) -> Result<(), WorldError>;
    fn whole(&self, archives: &[Section], out: &mut Section) -> Result<(), WorldError>;
    fn link(&self, archives: &[Section], whole: &Section) -> Result<Index, WorldError>;
    fn placements(&self, index: &Index, map: MapId, out: &mut Vec<PlacementRow>) -> Result<(), WorldError>;
}
pub trait MapReader: Sync {
    fn read(&self, index: &Index, map: MapId, worker: &mut Worker) -> Result<Arc<dyn MapBody>, WorldError>;
    fn interior(&self, index: &Index, map: MapId, body: &dyn MapBody, placed: &InteriorPlaced,
        worker: &mut Worker) -> Result<Arc<dyn MapBody>, WorldError>;
}
pub trait MapBody: Any + Send + Sync {
    fn nodes(&self) -> &[Node];
    fn draws(&self, at: &Moment, out: &mut Vec<Draws>);
    fn lights(&self) -> &[Light];
    fn occluders(&self) -> &Occluders;
    fn interiors(&self) -> &[InteriorPlaced];
    fn lay(&self, drawn: &Drawn<'_>, at: &Moment, out: &mut Instances);
    fn bytes(&self) -> u64;
}
pub trait ModelReader: Sync {
    fn models(&self, index: &Index, asks: &[AssetKey], worker: &mut Worker, out: &mut Vec<(AssetKey, Model)>);
    fn textures(&self, index: &Index, asks: &[TextureAsk], worker: &mut Worker, out: &mut Vec<(TextureAsk, Texture)>);
}
pub trait WorldLook: Sync {
    fn switches(&self) -> Vec<SwitchKind>;
    fn frame(&self, at: &Moment, out: &mut dyn Feed) -> FrameFacts;
}
pub trait EditTitle: Sync {
    fn operations(&self) -> Vec<OperationKind>;
    fn open<'w>(&self, place: &EditPlace<'_, 'w>) -> Result<Box<dyn EditSession + 'w>, EditError>;
}
pub trait EditSession: Send {
    fn guard(&self, at: &Address) -> Guard;
    fn apply(&mut self, op: &Envelope) -> Result<Applied, EditError>;
    fn body(&self, map: MapId) -> Option<Arc<dyn MapBody>>;
    fn flush(&mut self) -> Result<Vec<Written>, EditError>;
    fn base(&self, at: &Address) -> Option<Base>;
}
pub struct Envelope { pub kind: OperationId, pub at: Vec<(Address, Guard)>, pub body: Box<[u8]> }
pub enum Guard { Unwritten(u64), Written(Box<[u8]>) }

// model.rs
pub enum Gate { Always, Held { switch: Option<SwitchId> }, When { hours: u32, weathers: Vec<u64> } }
pub enum Draws { Model(AssetKey), StandIn, Nothing }
pub struct Instances { pub models: Vec<AssetKey>, pub node: Vec<NodeId>, pub model: Vec<u32>,
    pub rows: Vec<[[f32; 4]; 3]>, pub sphere: Vec<[f32; 4]>, pub fade: Vec<f32>, pub words: Vec<u32>, .. }
```

Beside these, `model.rs` holds `MapId` (made by a link, never stored), `MapRow` (a name, a label, its boxes, its parents, its gate and its archive), `Drawn` (a body's key, its drawn entities, its drawn lists and the instances left out of them), `AssetKey`, `TextureAsk` (a scope, a name and the pixels wanted), `Moment`, `Switched`, `Index` (`with_title`, `title`, `named`), `InteriorPlaced`, `Light`, `Occluders`, `Model` with its `Lod`, `Mesh`, `VertexLayout` and `Surface`, `Texture`, `FrameFacts`, the `Feed` trait with `FeedName`, `PlacementRow` and `interior_key`; `contract.rs` holds `Opening` (the configuration folder, the `Keys`, a cancel and a progress), `Worker` (a reading thread's state between reads), `EditPlace` (the opened world, its index and a folder where the store is one), `Applied` (the inverse and the maps changed) and the error types; and the modules `caps`, `policy`, `slots`, `sentence`, `pose`, `driver`, `kept`, `kit` and `fake` hold the rest.

The build changed these parts; the others stand as the plan of 2026-11-26 set them: a world is a stack whose level is an option and never a switch, so two levels of one install are two sessions and two kept files (`WorldSpec::identity`); the kit opens a hand-built install with `Keys::None`; and an ask is a scope and a name, in batches the adapter groups by dictionary or pack.

| Part | What it is, and why |
|---|---|
| A game | An id, `GameId`, and never `Game` of `or-title`, which only a process-wide registry makes and a fake title must not enter. The core has no edge to `or-title`; the registry crate maps a registered game onto the id |
| The index | Every reader is handed the linked `Index` its ids are rows of, and a title's own linked tables ride on it (`Index::with_title`), so an install keeps no hidden state from its last link. `link` is called after every load, from nothing but the sections |
| The body | What a node draws is the title's, so the core asks a body once for what draws every node (`draws`: a model to wait for, a stand-in, or nothing), selects nodes, and calls `lay` once a body a selection, with its drawn entities, its drawn lists and the instances left out of them. Nothing of a title is called once a node. A body is `Any`, so a title finds its own type in one handed back. Its lights and occluders reach the host with the selection |
| An interior | A body of its own under `interior_key`, read from the body that places it, which is an edit session's after an apply: the instance's place is read from that body, never from the install. A title that reads interiors with the map states no `InteriorPlaced` |
| An instance | Fixed columns the host culls and picks by, each model listed once, and a block of the title's words of the width `WorldCaps` states |
| The install | A title's `open` returns a world it owns; one made over values a caller lends is driven for as long as they are lent, a lifetime `Driver` and `EditPlace` carry. Capabilities and policy are the install's, since Legacy and Enhanced differ under one title |
| The edit store | Opened over an opened world, lent to it. After an apply the edited map is neither in the mount nor on disk, so the driver asks the session for a changed body first. An envelope addresses a set of files, each with its guard, and its inverse may restore bytes. The caller keeps the steps, through a trait that is R71i's |

### The driver and the kept file, as built

`Driver` of `crates/domain/or-world/src/driver.rs` is a synchronous machine around `Forest`, and it is `Send`:

- **Its calls.** `want` gives the maps a view wants whose body is to be read, nearest first, taking from the edit session any it holds. `give` hands a body or a refusal back. `settle` puts what came in into the forest and reads nothing: it lists the interiors it waits for, which the caller reads from `placing` and hands back with `give_interior`. `stream` does all three on the caller's thread. `select` returns the instances, the models waited for and the bodies in the forest.
- **Reads may come back late or twice.** A read for a map the session holds is ignored, as is a refusal for a map whose body is held (`a_refusal_given_after_a_body_leaves_the_body_in_the_forest`); an interior read from a body an edit has replaced is dropped and asked for again.
- **What is open in it.** Wanted maps are found by a scan of every row, where GTA V's forest asks a box tree; over 200 maps of 500,000 nodes, all drawn, a select took 346 ms against the forest's 196 ms, on a loaded machine. An interior's refusal is kept by the interior's name for the world's life, which is wrong for a refusal that depends on the instance. Interiors' bodies are not counted in the budget. A map the session holds enters the forest afresh at each apply, so its far model fades in again.

`crates/domain/or-world/src/kept.rs` is the one kept-file mechanism: a head (the title's id, the world's identity, the version of the adapter's rules, a stamp for each input), a section of rows for each archive and one derived across them. `refresh` reads only the archives whose stamp changed, and answers absent, current, changed, foreign or refused. ⚠ An archive with no stamp is read at every open; archives in another order are not read again, but the derived section is made again; and the file's name is a hash of the world's identity, which differs between platforms for one path.

### What is still to come

- **The world thread and the reader pool**, the shell a screen wraps the driver in: R71b's parts 2 and 4.
- **The render profile**, which `WorldInstall` has no method for: R71f. `Capability::OwnPrograms` is data until then.
- **The kit's second tier**, over an install on disk, and **a height of the ground and a collision query**, capabilities with no method yet: R71d.
- **The envelope's ordering rules, the keeper of steps, comparison, put-back and merge**: R71i. The scales by category stay constants of the forest.

What stays out of the contract: the mount and the address of a body; the records an edit changes, which are the title's on its own file; the title's day beyond the frame facts, which reaches its programs through the feed by name; what a body lays out beside entities; and the project, a finding of a core rule being the core's own row that a caller maps onto `or_project::content::rule`.

## How a game differs

Shared code holds no test of which game it serves: a game differs at three levels, and a test fails when shared code names one ([the developer's experience](#the-developers-experience)).

### Capabilities, as data

The core's `Capability` has the seven values below; what is chosen at open is an option and what is switched while open a switch, both slots. A panel section, a tool or a verb whose capability is absent is hidden or greyed with the reason.

| Capability | GTA V | GTA VI | RDR2, expected |
|---|---|---|---|
| A height of the ground | yes, from the bounds `crates/domain/or-depends/src/world_collision.rs` streams | yes, from the install's height map (`ground_under`) | unmeasured |
| A collision query with hits and overlaps | yes: `crates/domain/or-world/src/collision/` | no | unmeasured |
| Placement rows and find | yes: `crates/domain/or-depends/src/find.rs` | rows for one archive; no search | no |
| A weather gate on what streams | yes | none found | unmeasured |
| An edit store | yes, in a project | yes, over a document | no |
| A base to compare with, put back to and merge against | yes | no | no |
| Its own programs | Legacy after R71f, Enhanced after R71g | after R71h | not in this programme |

### Policies with defaults

A number is a field of `StreamPolicy`, a scale by level a table of it, and a rule asked once a map a function it holds; nothing of a policy is called once a node. The core writes the default and a game overrides what it disagrees with:

- **When a parent waits for its children.** A GTA V parent waits for exactly the children it states (`Children::Stated`); a GTA VI parent's count is raised to those loaded, which is the default.
- **When a map streams in.** GTA V's rule is its box grown by `PRESTREAM` metres (`streams_in`); GTA VI's is a sphere and a tier distance (`MapBox::wants`), the default.
- **How a level fades.** GTA V fades every level over one distance; GTA VI reads its distances from the install.
- **Parents and categories.** A GTA V map has one parent and its nodes one category; a GTA VI map has a list of parents with a selector for each element, and its nodes a category read from load flags.

The constants of `crates/domain/or-world/src/interior_portals.rs` are GTA VI's build rules and move to its adapter with `crates/domain/or-world/src/streaming_rules.rs`; the portal walk and the seven levels of `Level` stay.

### Extensions in slots

The core calls a slot and does not know who filled it (`Extras`: layers, rules, panel sections and streamed extras; operations are the edit store's), each shown by a `Sentence` the app's catalogs hold:

- **GTA V**: the layers of [R70f](Roadmap.md#r70f-layers-of-what-the-game-hides); its three vocabularies, the manifest and the first edit that imports; the rules of `crates/domain/or-maps/src/rules.rs`.
- **GTA VI**: the lane graph, map lights, places and the occluder tier as layers; the selector of a parent map and prop and grass lists as operations; the link and child-count check; and cards, grass, batch levels, time archetypes and room visibility as streamed extras.

RDR2's column is still unmeasured: R71m found what its world files name and not what they hold. What they name the contract states; a need found later is a change made before the freeze, with the kit as its check.

## Drawing

The owner chose option C: one shared frame host in which each game runs its own shader programs, with a plain fallback for what cannot be translated ([the record](Project_Context.md#each-game-draws-its-world-with-its-own-shader-programs-in-one-shared-frame-host-2026-11-26)). The figures of [R71m](#the-measures-that-decide-the-design-r71m) are in [what the measures found](../reference/Crate_Reference.md#what-the-measures-found) and [what a build's programs come to](../features/GTA_VI_Xbox_Build.md#what-a-builds-programs-come-to).

### The host

The host is in `or-renderer` and owns the mechanics once: device pages and the texture pool, instancing, culling and occlusion, the streaming budget, render targets and the scheduling of passes, picking and read-back, and the editor's overlays, which draw in a neutral pass after the game's frame, so they work for every title and in the fallback.

Most of it exists in the world files of `src/render/viewport/` and in `crates/runtime/or-renderer/src/world.rs`, and is moved, typed on the core's model; `src/render/viewport/world_occlusion.rs` names GTA VI's occluders today. What does not exist is pages that hold a game's own vertex layout, a pool bound by a game's sampler names, an id pass to pick by, and a scheduler of declared steps.

### The render profile

A profile is what a game states so that the host can run its programs: data and pure functions in the adapter, with no type of the graphics library. It is not in the contract yet; R71f adds it to the opened world, and a world that states none draws in the fallback.

- **Where its programs come from**, the translator that lowers them, by name, and the key a lowered program is cached under.
- **What it needs of the device**, in the shape `device_needs` of `crates/runtime/or-renderer/src/game_shaders/meshlets.rs` has; a device that lacks it gets the fallback and the reason.
- **Its frame**: named targets with their formats, and a list of steps over them, each of a kind the host knows.
- **How a material binds**: which programs draw it in which pass, with its blend, depth and cull state, or a refusal with its reason.
- **Its feed**: each frame, the values of its constants, by reflection name for GTA V and by binding and name hash for GTA VI (`FeedName`).
- **How a fallback surface joins its frame**: by name, the host's encoder that writes one into its targets. A profile that names none takes the fallback for the whole frame.

GTA V states a vertex buffer laid out by the vertex stage's inputs, a material as variables and samplers by name, and four colour targets with a depth and a stencil of material ids; GTA VI states vertex data and meshlet records in storage buffers, a material as a record of words with a heap of textures, and up to four targets with a float depth and no stencil.

**The frame is six kinds of step.** `crates/runtime/or-renderer/tests/the_frame_as_targets_and_passes.rs` lists the 29 calls `render_deferred` records, in their order, for both frames it draws, so a declared list can be held to it call by call. The kinds: a scene's surfaces by its materials' programs; the host's buckets of instances; one program over the whole target; one program over a kept mesh, once a record of a list; a copy; and a compute pass read back late. GTA V's frame takes five and the World screen all six. A step also states a condition on the frame's values, the variant of a technique they pick, a stencil reference, a blend constant, and whether it keeps or clears its target.

⚠ "Data, not code" has a counted exception, five passes that are a step of no kind: the exposure's meter, which is GTA V's frame's one, and the World screen's shadow cast by tile, rooms' lamps, terrain shadow and history. A dome and light volumes are a kept mesh and a list, which a kind states. Each of the five is a named pass in the renderer's module for its title, counted by a ledger a test holds; all five are in `or-renderer` today ([ruling 5](#asked-of-the-owner)).

The translators stay tools: the profile names its translator and the app hands the host one for each name, as `src/state/gta6_shader_translator.rs` and the helper process of `src/state/game_pack.rs` do today.

### The fallback

What cannot be translated, or what a device cannot build, is drawn in the fallback, and the panel lists what fell back and why. The neutral fallback is the host's own shader over the `Surface` a model states for each mesh. The fake title, GTA V and a new game draw with it; no port of GTA V's materials into it is built.

- **How it composes.** In GTA V Legacy's frame a fallback surface and a game's program already fill one G-buffer, depth and stencil, and one lighting pass lights both (a test of `crates/runtime/or-renderer/src/world/tests.rs` that needs a baked pack and a device, and so runs in no gate). Each brings its own vertex buffers and pipeline; what is shared is the targets' encoding and the stencil's material. So in GTA V a material that refuses falls back alone; GTA VI's frame has no such encoder until R71h writes one, and a refusal there puts the whole frame in the fallback ([ruling 13](#asked-of-the-owner)).
- **GTA VI's is richer and stays.** Its ten families are a hand port of what its programs do: the rejected option A, already built. The bucket is neutral and stated once, in the core; the family is GTA VI's, read by the renderer's GTA VI module from the material record a model carries, with the mapping `src/render/viewport/world_material.rs` holds today. The families gain none. Until R71h they are GTA VI's whole frame, and that frame's shadow, occlusion, lights, sky, fog volumes, distant lights, coronas and resolve are borrowed from a pack baked from a GTA V Legacy install, which the architecture test lists step by step ([ruling 4](#asked-of-the-owner)).
- **It is drawn on every gate.** A session has a fallback-only switch, and each title's shots script takes one shot with it on. The fake title has no material that refuses yet ([ruling 17](#asked-of-the-owner)).

### What the graphics library cannot run

- **Hull, domain and geometry stages.** wgpu has none, and GTA V's tessellated and instanced technique variants bind them; the plain techniques build almost whole ([measured](Shader_Pipeline_Design.md#what-a-pass-needs)). A material whose only technique needs such a stage falls back, and R71f measures the two ways to place an instance before it builds one.
- **GTA VI's passes stop at what the game fills** (2026-11-27). Of 26,698 program blocks 26,616 translate, but 20 of 22,010 two-stage passes bind with the plain binder: the rest stop at a buffer the game fills at run time, never at the translator. Of the 748 pairs that shade a model the binder refuses 272: 124 place instances by records the game fills, 81 pick a resource by a number it sets, 55 loop to a count it fills.
- **GTA VI on Metal: most, not all** (2026-11-27). On the owner's Mac a Metal device created and drew with 413 of the 476 pairs handed to it. Of the 63 refused, 52 bind more vertex-stage buffers than Metal takes, which a fitting no device has built answers, and 10 fail Metal's compiler. No frame of a real model was drawn there through the app.
- **GTA VI's lighting** (2026-11-27). Its technique translates in 4 of 4 effects and is not run: 64 of its bindings are filled by the game alone and known by hash alone, and no frame schedules the passes that fill them.

### Where translated programs live

Translated programs come from the user's own install, are cached in the configuration folder and never enter the tree. The cache becomes keyed by game and install, where today's GTA V pack is one folder for the machine, and R71f adds every cache's extensions to the provenance lint and the ignore file, which name the effect files alone today.

## The app

The app has one world, three times over:

- **One session**, in `src/state/`, keyed by the world it opened (game, install, options, layers), held by a handle on the flat `AppState` (`the_shell_is_flat_rather_than_deep`). It owns the opened install, the camera, the switches, the world thread and the reader pools, and no static holds a world: an install's tab and a project's tab are two sessions, as [R70c](Roadmap.md#r70c-the-gta-v-world-streamed-and-drawn) asks.
- **One bridge**, in `src/render/viewport/`, that hands a session's selection and feed to the host and names no game.
- **One set of panels**, whose sections follow the capabilities and the slots. Of the World panel's sections, Time and weather, Camera, Capture and Streaming are neutral; What streams becomes the switches as data, Mount the kept file's report, and the blocked section's sentence the adapter's key.

### The command line

A neutral verb takes a world as a screen does: an install, the game where the folder does not decide it, the title's options and its layers. A renamed verb keeps its old spelling for one release as a refusal that names the new one.

| Verbs | What becomes of them | In |
|---|---|---|
| `world index`, `at`, `ground`, `find`, `select` | neutral, for any registered title, by capability | R71d |
| `world info`, `check`, `extents`, `manifest`, `partition`, `create`, `new-map`, `states`, `prefab`, `to-xml`, `from-xml`, `archetypes` | GTA V's file verbs: unchanged, then spelled under the title's name | R71l |
| `world delete-entity`, `delete-map`; `maps apply`, `delete`, `edit-game`, `collision`, `roads`, `doors`; `rpf map-entity` of `crates/tools/or-cli/src/gta6_map_entity.rs` | edits: one apply verb that takes any title's operations through its store; a verb that only spells one operation refuses with its new spelling | R71i |
| `maps changes`, `merge` | by the capability of a base, which GTA V alone has | R71i |

## The developer's experience

- **The conformance kit**, `crates/domain/or-world/src/kit.rs`: 20 rules, each one a title can break, and `conformance_kit!`, which writes a test for each under the rule's name. That a held map is read once and a body laid out once a selection is no rule of the kit, since no title can break it: the driver's own tests hold it over the fake, which counts.
- **Two tiers.** The first is built. The second, over an install written to a scratch folder, is R71d's, through the fixtures of `crates/domain/or-depends/tests/`.
- **The fake title**, the public module `or_world::fake`, which registers no game: five maps held in memory, a cube, a flat texture, a store with one operation, and 25 faults, each breaking one rule (`each_broken_fake_breaks_the_rule_it_was_built_to_break_and_no_other`). It has no render profile.
- **Architecture tests**, in `crates/domain/or-world/tests/the_core_names_no_game.rs`: the core names no game outside two ledgers, depends on no crate of its own layer or above, and no adapter's manifest lists another adapter or a crate above the domain. The ledgers are lexical and only shrink: an entry that no longer breaks the rule fails the test. Still to write, with their ledgers: that the host's and the app's world modules name no game outside those named for a title, and that no static holds a world.
- **Sentences.** A `Sentence` is made by `sentence!` alone, and the language gate walks the core's source and every adapter's (`a_sentence_a_world_adapter_names_that_no_catalog_holds_fails_the_gate`).
- **Checks that run.** A row that must see a frame names `just self-check --viewport` in full and adds a test of `or-renderer`. A row that moves a suite re-keys the slow tier of `.config/nextest.toml` and the list of `crates/tools/xtask/src/slow_tier.rs` in the same commit.
- **What adding a game costs.** A profile and a mount in `or-title`, readers in `or-io`, one adapter crate with the kit's macro in its tests, one registry line, and its keys in the catalogs; a test holds that list, as `crates/title/or-title/tests/adding_a_title.rs` does. Where it runs its own programs: a translator among the tools, and a module of `or-renderer` for the passes no kind states.

### What an adapter writes to pass the kit

One test file that holds `or_world::conformance_kit!(fixture());`, with `tempfile` among the crate's dev-dependencies, and a `fixture()` that returns the kit's `Fixture`: the title, the world to open, a `Scene` and, exactly where the capabilities state an edit store, an `EditFixture`. ⚠ Every rule opens the world through the title's own `open`, with no key, so the title must open a world built by hand: in memory, as the fake's, or written to scratch.

The scene is stated in the title's own map names and camera positions: an eye where nothing streams; a weather; a parent map and a child map, with a far eye where the parent draws alone and a near eye where every child draws whole, the two with different models; a map by the clock, with an hour on and one off; a held map with its switch; an eye at an interior, at a list and at a light and an occluder, each with its count; and a placed map with its count of rows.

Beyond the scene, the kit holds the adapter to this:

- **The index.** `link` builds the same rows from the sections alone on a second open that reads no archive. A child map's row names its parent map, a node's parent names a map by `MapId::key`, and a gate is data on the row.
- **Bodies and models.** `draws` pushes an entry for each node. `lay` pushes exactly the words an instance the capabilities state. Every model a place of the scene draws with reads, with a level of detail, whole vertices, indices in range and each texture its surfaces ask for. The look states a whole frame at every hour.
- **Capabilities.** Refused: a base without an edit store, a store without an edit fixture, a base answered where none is stated, and placement rows where the capability is absent or other than the scene counts. Options, switches, slots and operations are each listed once, and the policy states every level.
- **Edits.** ⚠ A session's `body` returns the same value until the next apply and another after it: the driver compares by pointer, so the same one after an apply hides the change, and a fresh one each call puts the map in the forest afresh at every view. A reused envelope is refused as stale, and an operation with its inverse restores every guard.

## The order of work

A row is one programme part, lands in the parts its brief lists, and ends on a check that failed once before it passed. A row starts when the rows it depends on have merged.

| Row | Content | Depends on | Stands |
|---|---|---|---|
| R71a | The contract, the kit and the fake title | | Built. Left: the world thread and pool, the render profile, the kit's second tier |
| R71m | The measures that decide the design | | Taken. Left: the share by family, a model's frame on Metal, the lighting's bindings, RDR2's mount order |
| R71d | The GTA V world behind the contract, over its crates as they stand; the registry; the neutral verbs | R71a | |
| R71b | The GTA VI world behind the contract, in five parts | R71a; from part 2, R71d | |
| R71c | One session, one host, panels by capability, in four parts | R71b | |
| R71e | The GTA V world drawn in the fallback pass | R71c, R71d | |
| R71n | The GTA V files moved into the adapter | R71d; the finished branches of R70 landed | |
| R71f | The render profile, and the programs of GTA V Legacy | R71e | |
| R71g | The programs of GTA V Enhanced | R71f; the Enhanced inventory of [R58f](Roadmap.md#r58f-shader-baking-off-windows) | |
| R71h | The programs of GTA VI over a world | R71c; R71f for the kinds of step | |
| R71i | One edit stack | R71b, R71d | |
| R71j | The world tools built once | R71c, R71e, R71i | |
| R71k | Red Dead Redemption 2 | [R56](Roadmap.md#r56-rdr2-the-second-container) for the formats | |
| R71l | Leftovers and the ledger | every row above | |

- **R71d and R71b's first part run together**, and R71i beside the drawing rows.
- **R71d lands before R71b's second part**, which rebases its lines of the workspace manifest, the lock file and the registry. So GTA VI's own loop is deleted only after both adapters call the core's driver.
- **R71a edited two files the branches of R70 change**: in `crates/domain/or-world/src/streaming.rs` how a parent waits became the enum `Children`, which `Forest::waiting_for` takes, and the constructor call in `crates/domain/or-depends/src/world_forest.rs` passes `Children::Stated` ([ruling 7](#asked-of-the-owner)).

Rules for every row: no window is opened, and a screen is checked on `just shots` and `just self-check --viewport`; documents travel with the code; a moved test keeps its body; and a format's read path changed by a move runs that format's slow suite.

### The contract, the kit and the fake title (R71a)

Built in `crates/domain/or-world/`, with no file moved. [The contract](#the-contract) is its outline, [the driver and the kept file](#the-driver-and-the-kept-file-as-built) what it does and where it is open, [what is still to come](#what-is-still-to-come) what it left, and [the kit](#what-an-adapter-writes-to-pass-the-kit) what the next adapter writes. It is held by `crates/domain/or-world/tests/the_fake_title_passes_the_kit.rs`, by `crates/domain/or-world/tests/a_broken_title_fails_by_the_rule_it_breaks.rs`, in which each fault breaks its one rule and fails that rule's test by its name, and by the driver's and the kept file's own tests, each seen red under a break of the code it holds. GTA V's forest passed its install suite on both installs on 2026-11-27, in the integration lane.

### The measures that decide the design (R71m)

Taken on 2026-11-27 on the owner's Mac, with no code path of a screen or a verb changed. The figures and their commands are in the two documents [Drawing](#drawing) links, and what follows from them is in that section and in [Red Dead Redemption 2](#red-dead-redemption-2). Left, each with why: the share of programs by the fallback's families, since a family is read from a material record in the app; a real model's frame on Metal, and a fitted pass built on a device; the names of the lighting's 64 bindings; the order RDR2's archives mount in, which no row states; and what its models hold, which nothing reads. ⚠ The frame's test holds the calls `render_deferred` records and their order; the kind, the program and the count of a step are a reading entered by hand.

### The GTA V world behind the contract, and one kept file (R71d)

Read [the contract](#the-contract) with `crates/domain/or-world/src/fake.rs`, an adapter whole in one file, `crates/domain/or-depends/src/world_index.rs` with its folder, `crates/domain/or-depends/src/world_forest.rs`, `crates/title/or-title/src/vfs/game_mount.rs` and the fixtures of `crates/domain/or-depends/tests/`.

- **Builds.** or-world-gta5 as a new crate over the core, `or-depends`, `or-io` and `or-title`. What the kit asks of it is in [what an adapter writes](#what-an-adapter-writes-to-pass-the-kit).
  - *Its title and indexer*: an id, both games' ids, the options (the level by the mount's cutoff, story or multiplayer), the layers through the mount's options, the keys of the opening; rows for each archive, a link that leaves the mount's own tables on the index, and a version of its rules.
  - *Its map reader*: that of `crates/domain/or-depends/src/world_forest/read.rs`, as a body that states its occluders and lights, says in one call what draws each node, and lays out its entities and lists; its interiors are bodies of their own, each read from the body that places it.
  - *Its policy*: `Children::Stated` and `streams_in`; the clock and weather of a map as a `Gate` on its row; its state groups as switches; its capabilities, with the words an instance carries.
  - *Its models*: what names a model and its textures, as asset keys and asks grouped by dictionary. ⚠ The kit's rule `every_model_a_selection_draws_with_reads_whole` asks that each model the scene draws with reads whole, with its textures. Names alone do not pass it, so this row brings forward from R71e the decode of a drawable to the core's model, for the models the hand-built install holds or gains; the dictionary's entry, the fragment, the second target, the dictionary of textures read once and the surface by sampler names stay R71e's.
  - *What the contract lacks and this row adds*: a height of the ground and a collision query as methods a capability gates; the kit's second tier, over the hand-built install of `or-depends`' fixtures; and the registry crate with its first line.
  - *The neutral verbs* of [the command line](#the-command-line). Before a verb changes, the output of `world select` at fixed cameras over both installs is captured for R71e.
- **Check.** The kit passes for GTA V in both tiers. At fixed cameras over the hand-built install and both real ones, the adapter's selection through the core's driver is the selection `WorldForest` makes: the one check that holds the core's driver to a driver it did not write, and where the cost of its scan of every row shows. Two worlds of one install at two levels share no kept file, and a second start opens no archive that did not change. The neutral verbs answer over the fake title in the default tier, and the suites of the verbs that exist pass with no body changed.
- **Must not touch.** Any file of `or-depends` or `or-maps`, and of `or-world` any but those of the core this row adds to.
- **Not known.** Whether GTA V's rows can go through the core's kept file by the public sealing of its own; if not, GTA V keeps its file until R71n. Whether an interior's refusal should hold for the world's life, which the first real interior that refuses decides.

### The GTA VI world behind the contract (R71b)

Read [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md), `src/state/gta6_worldview/mod.rs`, `select_asking` in `src/state/gta6_worldview/stream.rs`, `src/state/gta6_worldview/model.rs` with `meshes_at` of `src/render/gta6_drawable.rs`, `src/self_check/shots/compare.rs`, and the shots steps of [Development_Workflow.md](../guides/Development_Workflow.md#screenshots).

It lands in five parts, each on its own check:

1. **The reference, made able to fail.** A World shots script at fixed cameras, hours and modes, with `world settle`, `world converge`, a shot and a `world dump` at each; a comparison in which a missing reference fails and the share of pixels past 16 levels is bounded beside the mean; dumps compared over a stated list of fields; and an install-gated suite of digests of models, textures, the index's rows and the look's frame values, on the unchanged code. References live where [ruling 12](#asked-of-the-owner) says. It ends when an altered reference and an altered digest each fail, and is under way in a lane of its own.
2. **The closed move.** By the closure of imports at `9fdf0fee`, 13 files can leave the app before the selecting half. Seven change in imports alone: the reader pool, to the core, its workers keeping their open files in the core's `Worker`; `src/state/gta6_worldview/index_types.rs`, `src/state/gta6_worldview/lights_lod.rs` and `src/state/gta6_worldview/lights_index.rs`, to `or-io`; the lights' law and variables and the material, to the adapter. Six follow after two cuts: the index with its kept file, mount binding, scan and states, and the occluders. The cuts: the block of `src/state/gta6_worldview/index.rs` that implements a method of the app's `Viewer` (`land_and_water`) stays in the app, and the mount binding takes the configuration folder and the keys from the `Opening`. The adapter's title, its registry line and the slow tier's keys land here.
3. **The kept file.** The index goes onto the core's mechanism: rows for each archive with names unresolved, its tables derived across archives in the one section `whole` writes, and a `link` that makes the ids and leaves GTA VI's own tables on the index for its readers. `fresh_game.json` is one of the other inputs, with its stamp. The first open deletes the old kept file of that install, moves `fresh_game.json` from its old place and shows the rebuild's progress, whose time is recorded: a change of the rules' version costs every user that rebuild.
4. **The selecting half.** The two symbols it takes from the renderer move below it; then `src/state/gta6_worldview/stream.rs` with its five child modules, the interiors and the room walk move, the block of `src/state/gta6_worldview/stream_grass.rs` on `Viewer` staying in the app. The world thread is rewritten around the core's `Driver` and enters the core with the pool: `want` on the thread, the reads in the pool, `give`, `settle`, and the interiors it waits for read from the placing body and given back. Of `select_asking` the mechanics every title shares are the driver's already; GTA VI's rules (held codes, time archetypes, batch levels that hide a parent's instances, cards that hand off to trees, the walk that marks an instance as casting only) go to its rows' gates, its body's `draws` and `lay`, and its policy. The occluders and the lights leave their own loops and pools for the one read of a body. `crates/domain/or-world/src/streaming_rules.rs` moves to the adapter but for the two functions the forest calls.
5. **The outputs.** The 18 other files. The model reader returns the core's `Model`: the vertex decode of `src/render/gta6_drawable.rs` goes below the app, and `meshes_at` stays, calling down, with its skinning. The texture reader answers batched asks, and `src/state/gta6_texture_index.rs` goes with it. The sky, wind, water and terrain files return frame facts and feed the rest by name; their blocks on `Viewer` stay in the app. The kit's test file lands here, its rule on models needing this reader.

- **Deletes.** `src/state/gta6_worldview/index_keep.rs`; the GTA VI streamer's own loop around the forest; the occluders' and the lights' loops and pools; the block that opens an archive for a worker, written once in place of six times.
- **Check.** After each part the shots, dumps and digests of part 1 match, with no candidate line. No test of the viewer is deleted, and the pull request lists each restated. After part 3 a second start opens no archive and an edit of `fresh_game.json` rebuilds. After part 4 one caller of the forest's `select` is left outside tests. After part 5 [the kit](#what-an-adapter-writes-to-pass-the-kit) passes over a world built by hand, as the viewer's tests build rows and bodies. The tests of `src/state/gta6_edit_tests.rs` and the Map screen's pass with no body changed.
- **Must not touch.** What the picture is: no family, pass or constant changes. What the Map screen and the model viewport do, beyond the viewer's declaration in `src/state/gta6_world.rs` and the vertex decode part 5 moves. The singleton, which stays in the app as a shell over the crate until R71c.
- **Measured in part 1.** Two runs of the script agree in every digest and every compared dump field and not in the far band of their shots, which follows what the device holds: the digests and the dumps are the exact check, and each shot has bounds of its own ([the reference](../guides/Development_Workflow.md#the-reference-of-the-gta-vi-world-screen)).
- **Not known.** Whether the core's driver serves GTA VI's chunks, swaps and bands without a second driver, which part 4 answers by deleting its own, and what its scan of every row costs there. Whether `or-io` can write a map body with its registration documents for the kit's second tier.

### One session, one host, panels by capability (R71c)

Read `src/render/viewport/world.rs` and its neighbours, `crates/runtime/or-renderer/src/world.rs`, the five world viewer panel files, `src/self_check/shots/world.rs` and `src/ui/panels/status_bar.rs`.

It lands in four parts, each on the shots, dumps and digests of R71b:

1. **The session** of [the app](#the-app); `viewer()` gone from every call site and the shots steps addressed to a session; the five single-valued statics made fields of a session or of the opened install; the fallback-only switch, with its shot added to the script; the core's `Pose` in place of the panel's own; the architecture tests of the host and the app, with their ledgers.
2. **The host.** The bridge's mechanics moved to `or-renderer` a module at a time, beginning with `src/render/viewport/world_budget.rs` and `src/render/viewport/world_pages.rs`; occlusion typed on the core's occluders; the selection's instances carrying their node; `src/render/viewport/world_material.rs` moved to the renderer's GTA VI module; the bridge reduced to handing a session to the host.
3. **The panels and the preferences**, by capability and slot, the old preferences read once.
4. **The fake title drawn**, by the self-check and by a test of `or-renderer` that fails when the frame paints nothing, the renderer's test packs made reachable outside its tests.

- **Deletes.** The static viewer and the five process-wide values; the app's bucket, stated once in the core.
- **Check.** Two sessions opened in one process share no exposure, radiance table or night map, in a test that fails on a static. `just self-check --viewport` draws the fake title's world in the neutral fallback; its refusing material waits for R71f ([ruling 17](#asked-of-the-owner)). The architecture test's ledgers for the bridge and the panels are empty. The French screens of the world hold at the smallest window.
- **Must not touch.** The frame: `render_deferred` is still the one frame, and the fallback's families are unchanged.
- **Not known.** Whether overlays drawn after GTA VI's temporal pass carry the frame's jitter; a shot with a line over a still world answers it.

After part 1 the other GTA VI sessions resume ([ruling 10](#asked-of-the-owner)) on every file but the world files of `src/render/viewport/`, the world viewer's panels and preferences, and `src/self_check/shots/world.rs`.

### The GTA V world drawn in the fallback pass (R71e)

Read `crates/runtime/or-renderer/src/world.rs`, `crates/runtime/or-renderer/src/game_shaders/drawable.rs`, and [the Gen8 and Gen9 section](../reference/Crate_Reference.md#gen8-and-gen9-enhanced-is-a-separate-target) of the crate reference.

- **Builds.** The rest of the decode R71d began for its fixture: a drawable, a drawable dictionary's entry and a fragment to the core's model, for both targets, each by its file's own RSC7 version; a texture dictionary read once for the asks it holds; a material's fallback surface by its sampler names. The World room for a GTA V install, read only.
- **Check.** A synthetic world of parent and child levels draws the level the rules select at each distance, in real frames with no window. Over each install, at R71d's cameras, the instances drawn are those of the file R71d captured, and every model not drawn has a named reason. The frame over the city is measured on the owner's machine and its budget recorded. The data contract is frozen here, citing what R71m found of RDR2.
- **Must not touch.** The Shaded viewport of one drawable; GTA VI's picture.

### The GTA V files moved into the adapter (R71n)

Read the manifests of `or-depends` and `or-maps`, and `crates/domain/or-depends/src/resolve.rs`.

- **Builds.** One mechanical commit on `main` between waves, with no other lane open on the three crates. The GTA V modules of `or-world` move with `git mv`. The splits: `crates/domain/or-world/src/collision/bound.rs` and the half of the collision set that faces a bound go to the adapter, and the box type is lifted out of `crates/domain/or-world/src/extents.rs` into the core; the placement convention of `crates/domain/or-world/src/transform.rs` goes, and `scale_of` and `placed_corners` stay; the scene's types go to the core and its builder stays in `or-maps`. The world half of `or-depends` moves with the catalogue, the tables, the contents, the two types of the resolver its index names and `crates/domain/or-depends/src/tests_support.rs`, so that the resolver left behind imports the adapter and no ring forms; the script forms of `crates/domain/or-maps/src/script_text.rs` and the filename rule of an area's bounds go with it, and the place parser and the manipulator go to the core, which ends the edge from `or-depends` to `or-maps`. `WorldForest` becomes a thin wrapper over the core's driver with its public surface as it is, GTA V's index is kept by the core's mechanism, and each file moved leaves the ledgers of the core's architecture test.
- **Deletes.** `crates/domain/or-depends/src/world_index/kept.rs`; GTA V's own loop inside `WorldForest`; the include by path of `or-depends`' fixtures into `or-cli`.
- **Check.** The tests of `crates/domain/or-depends/tests/the_forest_of_a_mount.rs`, `crates/domain/or-depends/tests/the_world_index_of_a_real_install.rs` and the world suites of `or-cli` pass with no body changed, none deleted. `cargo tree` shows no ring. One world kept-file format is left in the tree. The slow tier holds every install suite it held, under its new key.
- **Must not touch.** What a GTA V file reads or writes as.
- **Not known.** Which of the two GTA V deletions with renumbering survives; R71l settles it.

### The render profile and the programs of GTA V Legacy (R71f)

Read [Shader_Pipeline_Design.md](Shader_Pipeline_Design.md), `crates/runtime/or-renderer/src/viewport/game_frame.rs`, `crates/runtime/or-renderer/src/game_shaders/mesh.rs` with its scene, and `src/state/game_pack.rs`.

It lands in three parts:

1. **The frame as a list.** Add the profile to the contract, and to the fake a two-step profile with one material that refuses ([ruling 17](#asked-of-the-owner)). Restate `render_deferred` as targets and a list of steps of [the six kinds](#the-render-profile), each with its condition, variant, stencil reference, blend constant and whether it clears, held call by call to the list `crates/runtime/or-renderer/tests/the_frame_as_targets_and_passes.rs` keeps. Write the five passes no kind states as named passes with their ledger. Give a pass of the pack's manifest its render state, key the pack by game and install, and add every cache's extensions to the provenance lint.
2. **An instance of a game's program.** Measure a city view drawn with a draw for each instance against a lowering that reads the transform by instance, and build the one the measure favours. Lay the pages out by the vertex stage's inputs. A fallback surface needs no layout in common with them. Bind the streaming pool's textures by sampler name.
3. **The world through the profile**, with GTA V's day from the install's timecycle as its feed, which is the feed [R70d](Roadmap.md#r70d-the-day-sky-water-and-shadows-of-the-world) asks for.

- **Deletes.** `render_deferred`, once the list draws its frame; the one pack folder for the machine.
- **Check.** The frame drawn from the list equals the frame `render_deferred` drew, for a drawable and for the GTA VI world, before that function is deleted. A drawable in Shaded and the same drawable placed alone in the world at the same hour and weather agree within a recorded tolerance. With no pack, or with the fallback-only switch, the world draws in the fallback and says so; a material that refuses falls back alone, and the panel lists each with its reason. A translated module staged in a commit fails the hook.
- **Must not touch.** The helper's protocol; no C++ enters the editor.
- **Not known.** The cost of either way to place an instance, the first measure of part 2.

### The programs of GTA V Enhanced (R71g)

Read [Shader_Definition_Database.md](../features/Shader_Definition_Database.md) for the two containers and `crates/tools/or-shader-front/src/dxil/`.

- **Builds.** A bake that drives the Rust front end over Enhanced's containers into a pack of the same manifest, and Enhanced's profile over it.
- **Check.** The inventory R58f leaves open is recorded, with the share of Enhanced's programs that lower and that each backend's writer takes. Over an Enhanced install the world draws through its own programs, and what does not is listed with its reason.
- **Must not touch.** Legacy's pack and bake, and the helper's protocol.
- **Not known.** Everything: no bake has translated an Enhanced program, and until this row closes an Enhanced install draws in the fallback.

### The programs of GTA VI over a world (R71h)

Read [what a build's programs come to](../features/GTA_VI_Xbox_Build.md#what-a-builds-programs-come-to), [what the graphics library cannot run](#what-the-graphics-library-cannot-run), `crates/runtime/or-renderer/src/game_shaders/meshlets.rs`, `src/render/gta6_meshlets.rs` and `src/state/gta6_shaders.rs`.

- **Builds.** GTA VI's profile: its targets and steps; its surface pass over the host's pages as storage buffers; a transform for each instance, which nothing feeds today; the bindings only the game fills, named; the buffer fitting Metal needs, applied where the app builds a pass; and an encoder its fallback surfaces write its targets with, where today such a surface is drawn after the resolve. Then its lighting: by its own programs if the bindings the game fills can be named, and by named passes if not, which the measure makes the likelier.
- **Deletes.** A family of the fallback's shader once every supported backend draws every material that used it; the renderer's mapping of a material onto a family with the last of them.
- **Check.** A model drawn alone in Shaded and placed alone in the world agree within a recorded tolerance, on Metal among the backends. A map's materials draw through their own programs, and each that falls back is listed with its reason. The script's fallback-only shot still matches. The borrowed GTA V pack is gone from the ledger, or [ruling 4](#asked-of-the-owner) says it stays. The render profile is frozen here.
- **Must not touch.** A family any backend still draws with.
- **Not known.** Whether a pass fitted to Metal's buffer count builds on a device, and whether the app's feeds draw a real model there: a shots script of one model in Shaded answers both first. What the lighting's bindings are.

### One edit stack (R71i)

Read `crates/domain/or-maps/src/operation.rs` with its session, the two other vocabularies named in [GTA V](#gta-v), `crates/domain/or-maps/src/changes.rs`, `src/state/workbench/maps.rs`, and the three GTA VI entity edit files of `src/state/` with `crates/io/or-io/src/gta6/map_grow.rs`.

- **Builds.** In the core, beside the envelope, the guard and the session R71a built: the envelope's ordering rules; the trait of who keeps the steps; and the skeleton of comparison, put-back and merge over a trait that gives an entity's identity, which [ruling 16](#asked-of-the-owner) bears on. GTA V's store stays `or-maps`: its session behind the edit session's trait, answering a changed body to the driver, its steps kept by the project's History. GTA VI's operations are new, in its adapter, as pure functions over the sections of a body: place, move, turn, scale, retype, add and remove across linked bodies, each with its inverse, guarded by a digest of that body. The app's document owns GTA VI's bytes, keeps its steps and lends the body to those functions ([ruling 14](#asked-of-the-owner)). One apply verb on the command line for any title's operations.
- **Deletes.** The three GTA VI entity edit files of `src/state/`, `src/state/gta6_map_extents.rs`, the second history of the Map screen, and the verbs [the command line](#the-command-line) turns into refusals.
- **Check.** The kit's edit tests pass for both titles and the fake: every operation followed by its inverse leaves the store byte for byte, a stale guard refuses, an operation with no inverse fails a test, and after an apply the driver's selection shows the change with nothing flushed. The apply verb runs the same tests with no window. The tests of `or-maps` pass with no body changed. The GTA VI edit tests are restated with no `AppState`, none deleted.
- **Must not touch.** GTA VI's navmesh and collision edits, which stay document steps for their cost ([ruling 6](#asked-of-the-owner)).
- **Not known.** Whether a scanned body can serve as GTA VI's base for a comparison; until it does, its capability says no. Whether a map a session holds can stay in the forest across an apply, so that its far model does not fade in again.

### The world tools built once (R71j)

Read [World_Editor_UX_Design.md](World_Editor_UX_Design.md), `crates/domain/or-maps/src/gizmo.rs`, `src/render/viewport/pick.rs` and `src/ui/commands/target.rs`.

- **Builds.** The surface the screen rows of R70 stand on, for every title: the pick by an id pass, the selection outline and the manipulator in the overlay pass, the registry of layers and of panel sections, a command's capability in the command table, and placement rows as a capability. GTA VI's placement rows replace the Map screen's world store and its kept file.
- **Deletes.** `src/state/gta6_world.rs` and `src/state/gta6_world_fill.rs`.
- **Check.** On the fake title a click returns the node under the pixel, a drag applies one operation to its edit session, the next selection draws the node moved through the session's body, and Undo takes it back, in real frames with no window. A tool whose capability a title lacks is greyed and says why. The plan overlay of a GTA VI archive draws the same shots as before.
- **Must not touch.** What each tool does for a mapper, which the rows of R70 own.

### Red Dead Redemption 2 (R71k)

Read [what the tree holds of it](#red-dead-redemption-2), [R56](Roadmap.md#r56-rdr2-the-second-container), `crates/io/or-io/src/rdr2/` and `crates/io/or-io-verify/tests/rdr2_world_census.rs`.

It lands in three parts: a mount in `or-title`, which needs the order the game mounts its archives in and must not assume every row is a bare hash; the formats in `or-io`, which R56 gates; and the adapter, view only, in the fallback pass.

- **Where the formats start.** Its maps and type files name GTA V's map and type classes, so GTA V's model should serve once the metadata reader takes an RSC8 head. One body decoded by hand held GTA V's header with its later fields shifted by 8 bytes: one sample, a lead. Models, fragments and bounds need readers nothing has begun.
- **Check.** The adapter passes the kit, and an install draws with every model not drawn named.
- **Must not touch.** GTA V's mount: the function that mounts GTA V is not widened to serve RDR2.
- **Not known.** Every figure, and its capabilities: R56 warns that each estimate made before a format was measured was low. Three nested archives are sealed with a key the bundled snapshot lacks.

### Leftovers and the ledger (R71l)

- **Builds.** Nothing new. It settles the two GTA V deletions, spells GTA V's file verbs under the title's name, moves the conversions the renderer still holds for a title into that title's feed where a profile now serves them, and corrects every document that states the old shape.
- **Deletes.** Whichever of the two GTA V deletions loses, and every ledger entry no ruling keeps.
- **Check.** Every ledger of the architecture tests is empty or holds only entries a ruling names, and an entry added to one fails. The lines each duplicate of [what is written twice](#what-is-written-twice) lost are published as a table measured with `git diff --shortstat` between `9fdf0fee` and the closing revision.

## What it changes for R70

What is finished of [R70](Roadmap.md#r70-the-gta-v-world-editor) lands as it is, and each of its rows in the Roadmap says what this programme changes for it: the headless parts keep their checks, are reached through the adapter in R71d and move crate in R71n; the world drawn is R71c to R71f; the edit vocabularies stay GTA V's store under the core's envelope; and the screens are built once, on the tools of R71j.

Three statements of [World_Editor_Plan.md](World_Editor_Plan.md) are replaced: that the seam between the titles is declared in the app beside the GTA VI viewer, that a GTA VI world keeps its own World panel, and its ruling 9, which the owner's choice of option C answers.

## Risks

Each is answered in the row named:

- **The move breaks the picture**: R71b's shots, dumps and digests, by a comparison shown to fail.
- **The contract is shaped by one title.** It was built against a fake alone: GTA V answers it in R71d, before GTA VI's loop is deleted or the host is typed on the model.
- **A third driver is written and the two stay**: R71b's part 4 and R71n each delete their loop.
- **The local branches of R70 conflict with the move**, 33 of the 41 changing the three crates on 2026-11-27: R71d moves no file and R71n waits for the owner's list ([ruling 7](#asked-of-the-owner)).
- **The city costs more than the frame has, or the core's driver more than GTA V's**: R71d's comparison and R71e's measure.
- **GTA VI's programs stop at what the game fills**: R71h begins with the instance's transform, and what stays unfed falls back.
- **Nothing draws the neutral fallback once profiles work**: the fallback-only switch. **Game content enters the tree**: R71f's lint, and references kept outside the tree.
- **A capability becomes a test of the game by another name**: a branch on a capability that changes a shared rule is a policy, and is written as one.

## Rulings

The three made on 2026-11-26 are recorded under the links of the status line.

### Asked of the owner

Each has a default a row takes until a sentence reverses it. The measures answered 5 and 13; the build changed 1, 4, 7 and 12; 15 to 17 are new.

1. **A registry crate lists the titles**, where the agent guide says there are no facade crates; it also maps a registered game onto the core's `GameId`, the core having no edge to `or-title`. Default: yes; it re-exports nothing, as `or-content` for the kinds of content.
2. **`or-renderer` depends on `or-world`, and on each adapter whose module it holds.** Default: both edges; they point down the chain.
3. **The translators stay tools and the app hands them to the host.** Default: they stay.
4. **The GTA VI world keeps GTA V Legacy's pack until R71h**, and its families on a backend that builds none of its passes. The borrow is longer than the lighting first named: the sky, the fog volumes and the resolve are the pack's too. Default: all of it stays, in the ledger.
5. **No line that touches the device enters an adapter.** Answered by the count: the five passes no kind states are all in `or-renderer`. Default: yes.
6. **GTA VI's navmesh and collision edits stay document steps**, for their cost. Default: they stay out of this programme.
7. **R71n waits for the finished branches of R70**, which are the owner's to name. That R71a lands first is overtaken: in the integration lane of 2026-11-27 its commits stand above the lanes of R70 integrated there. Default: as stated; R71d, R71b and R71c wait for none.
8. **GTA VI gains no comparison, put-back or merge in this programme.** Default: yes.
9. **RDR2 is view first and edit later.** Default: yes.
10. **The other GTA VI sessions resume when R71c's first part merges.** Default: as stated.
11. **The core keeps the name `or-world`**, so R71n rewrites the imports of GTA V's model. Default: the name agreed, which R71a built on.
12. **Where reference shots and digests are kept, and where the pack a lit frame needs comes from.** Default: until the owner names a store, both are made again in scratch from the revision before the part.
13. **A material that refuses puts its title's whole frame in the fallback.** Answered for GTA V Legacy, where a fallback surface already fills the game's own targets; it stands for GTA VI until R71h writes an encoder. Default: alone in GTA V, the whole frame in GTA VI.
14. **The app's document owns GTA VI's edited bytes**, and the adapter's operations are pure functions it lends a body to. Default: the document.
15. **Whether an installed schema snapshot may lay out a class a document's own table neither states nor names**, or any class of a root built from nothing. A typed root that leaves its layout is laid out by code, then by its own table, then by the snapshot for such a class alone, so a machine with a snapshot saves what one without refuses, in 56 of 336 sampled Legacy maps; of 61 such among 352 Enhanced maps it saves one. An edit store of R71i inherits the answer. Default: yes, as today, held by `a_copy_of_a_class_an_rbm_file_neither_states_nor_names_is_laid_out_by_the_snapshot_and_refused_with_none`.
16. **The guid a new entity takes.** Today one more than its map's largest, so an entity added after the one with the largest guid went takes that guid, the comparison reads the two as one entity changed, and put-back replaces the added one with the removed, as a review of 2026-11-27 ran it. The ways out: a comparison that pairs by guid only where the archetype or the pose is kept, or an addition that takes a guid neither the file nor its base states. Default: the second, since the session holds the base.
17. **What the fake title refuses with before a profile exists.** R71c's last part is checked on the fake's refusing material, and R71a built the fake with no profile. Default: R71c draws the fake in the neutral fallback alone, and the refusing material and its check move to R71f's first part.

## Sources

- The three readings of the tree at `9fdf0fee`, of 2026-11-26, the two reviews of this plan against the code, of 2026-11-27, and the reports of the engineers who built and hardened R71a and took and hardened R71m, of that day.
- [World_Editor_Plan.md](World_Editor_Plan.md), [Shader_Pipeline_Design.md](Shader_Pipeline_Design.md), [Shader_Definition_Database.md](../features/Shader_Definition_Database.md), [GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md), [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) and [the layers](../reference/Crate_Reference.md#layers) of the crate reference.
