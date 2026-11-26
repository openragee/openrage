# World editor plan

What OpenRage takes from NodeRunner Studio for GTA V, what that rests on, how it fits the tree, and the order and the briefs of the rows that build it.

Status: the plan of 2026-11-25, written on the owner's request of that day ([recorded](Project_Context.md#every-noderunner-studio-capability-for-gta-v-is-wanted-in-openrage-2026-11-25)) and built in lanes since the owner said so the same day ([recorded](Project_Context.md#the-world-editor-plan-is-built-in-parallel-lanes-2026-11-25)); [R70](Roadmap.md#r70-the-gta-v-world-editor) says where each row stands.

## What this plan is for

The owner's words: every capability of NodeRunner Studio, for GTA V, in OpenRage, with the world editor and the project system integrated and an experience that suits a person who makes maps. The plan is three documents: this one; [World_Editor_Parity.md](World_Editor_Parity.md), which weighs each capability for and against and says [what each product gets right](World_Editor_Parity.md#what-each-product-gets-right) and [what is left out](World_Editor_Parity.md#left-out-and-why); and [World_Editor_UX_Design.md](World_Editor_UX_Design.md), which says what a mapper would meet, each part tagged with the row that builds it.

Three people use the result, and where their needs conflict the mapper's wins:

- **The mapper**, for a FiveM server or for story mode, who adds, moves and removes objects, edits an interior and ships a resource.
- **The script developer**, who needs positions, model names and hashes in the form a script takes.
- **The curious**, who wants to see how the map is made: its levels of detail, its roads, its scenario points, its collision.

## What it rests on

- **NodeRunner Studio at `1a04e46e`**, its commit of 2026-11-24 and version 1.9.0, read on 2026-11-25: the README, the specification its author keeps, the interface strings, the test names and the header comments of its sources.
- **Its licence is PolyForm Strict 1.0.0**, which permits reading and non-commercial use and forbids distribution and any work based on the software. OpenRage takes what a person can do with it and nothing of how it is written: no code, no text, no table and no asset ([Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources)). The three documents describe each capability in OpenRage's own words and repeat no interface string, and that no lane opens NodeRunner's sources is [ruling 18](#rulings-asked-of-the-owner).
- **The tree at `25af0f42`**, read on the same day: `crates/domain/or-world`, `crates/domain/or-maps`, `crates/domain/or-depends`, the mount of `crates/title/or-title`, the Map screen, the World screen that GTA VI has, the project container, and the rulings of [Workbench_Plan.md](Workbench_Plan.md#rulings-in-force).
- **Nothing here ran either program.** What NodeRunner draws was read, not watched, and no figure of its speed is repeated.

## Verdict

1. **The world is one screen with two doors.** The World room of the rail shows a GTA V install as it shows a GTA VI one, with no project. The Map content of a project shows the same world with the project laid over it, and that is where an edit goes. One viewport and one camera serve both, which changes the pointer of the two screens that exist ([ruling 8](#rulings-asked-of-the-owner)).
2. **The first thing to build is the GTA V world drawn with its models**, which the tree lacks: the Map screen draws boxes, and the streamed world is GTA VI's. [R70b](#the-gta-v-world-index-r70b), [R70c](#the-gta-v-world-streamed-and-drawn-r70c) and [R70g](#collision-as-a-query-r70g) build it, and the screen rows stand on them.
3. **A project keeps holding sources, and a changed game map is one of them.** The first edit of an object the game placed imports its map as a source that keeps the game's file beside it, where NodeRunner's project keeps its rewritten map alone and compares it with whatever the install holds that day. The kept file is a base that does not move, for the list of what changed ([R70y](#what-a-project-changed-and-putting-it-back-r70y)) and for a merge. Such a source is the game's content, which a repository leaves out by default ([ruling 17](#rulings-asked-of-the-owner)).
4. **One vocabulary of edits.** Each thing a tool does is a named operation with an inverse: a step of History, a verb of `openrage-cli` and, later, the message a second person receives.
5. **Rules run while the person works**, in the Problems panel, with a way to go to each finding. A check that runs only at export reports a mistake after the work built on it.
6. **The build is the project's build.** A FiveM resource and a story pack come out of the plan every other content uses. A map adds the manifest of its maps, the path a story pack streams maps from and, in its own row, the audio of an interior.
7. **Nothing here proves the game loads a file**, and no agent runs the game. So [R70a](#what-the-game-loads-r70a) is first: a recipe that builds a small resource for each kind of file a row writes, and a sheet the owner fills from a running game. Until a line of it is filled, the screen says "not checked in game" where the person builds.
8. **The plan puts everything in the app, not in a plugin**, which draws lines one pixel wide and no mesh ([Plugin_ABI.md](../reference/Plugin_ABI.md)). That would overturn the place [R47](Roadmap.md#r47-the-plugin-slate-after-the-first) gives the interior builder, the walk mode and the thumbnail browser, which wait for [ruling 2](#rulings-asked-of-the-owner).
9. **Left out:** the earlier games, an updater, a link to a running game, a rescue copy, writing the XML a Blender add-on reads, authoring navmesh, translucent windows and a panel as a window of its own ([what is left out, and why](World_Editor_Parity.md#left-out-and-why)).

## Where the tree stands

What a row can stand on, and what is absent. The paths are the ones to read first.

| Capability | In the tree | Missing |
|---|---|---|
| Reading and writing `.ymap` and `.ytyp` | The typed world family in `or_io::meta`, the views of `crates/domain/or-world`, a canonical XML that compiles back. `crates/domain/or-world/src/create.rs` makes a map, an entity, a type file and a base archetype from nothing, written as PSO. An interior's rooms, portals, entity sets and time cycle modifiers are typed (`crates/io/or-io/src/meta/typed/interior.rs`), a save refuses a broken interior, and a record copied from another file is conformed to the class this file states or refused by name | A record made from nothing in an RBM file, whose class would carry a signature word nothing derives, and an interior, a room or a portal made so; the Map screen still adds by copy. Some world files do not reach their XML fixed point ([R32](Roadmap.md#r32-world-data-ymap-ytyp-and-interiors)) |
| Moving, adding and deleting an entity or an archetype | `or_maps::edit`, with the level-of-detail links renumbered inside one file and the extents grown in the same step. `or_maps::operation` names each as an operation with an inverse that states what it applies to, and a session of it holds the sources of a pack open and writes them once, which the Map screen does for each pack it edits, writing at the save of the project (`src/state/workbench/maps_session.rs`). `crates/domain/or-maps/src/first_edit.rs` imports the map of an object the game placed as a source and edits it, taken back whole (`openrage-cli maps edit-game`) | An operation is no step of the project's History; a link into a parent map is left alone by an operation that takes one entity out, and renumbered only by a planned deletion; no occluder, parked vehicle, light, room or portal |
| Maps that load together, and the manifest of a set | `MapGroup`, `crates/domain/or-world/src/manifest.rs`, `openrage-cli world manifest`. A Maps content writes `_manifest.ymf` from the manifests of the game it keeps (`crates/domain/or-world/src/manifest_build.rs`), with a derived entry for a map none of them states, its flags by a rule every entry of two installs follows | The lists a derivation does not give, the entry an interior's type file is stated by, and a manifest for a content that keeps none |
| Creating and deleting a map with every link kept across files | `WorkingSet` in `crates/domain/or-world/src/execute.rs`. `crates/domain/or-maps/src/delete.rs` takes an entity out with its chain of far models across the map files a pack holds as sources, over the typed roots and with no schema snapshot, behind `openrage-cli maps delete` and the Map screen's Delete, which does the plan as one step of Undo | A map created or deleted is reached from `openrage-cli world` only, and written only where a schema snapshot is staged, which is no user's machine; a deletion reaches a map of the game that is not a source yet only as the first edit of an object, which no screen plans, and the Map screen shows no preview of the one it does |
| Which maps the game requests at an hour and a weather, or only by script | `crates/domain/or-world/src/state.rs`; the GTA VI World panel lists the script-managed groups | No screen uses it for GTA V |
| A 3D scene with a manipulator | The Map screen: Scene, Plan, Selection, `or_maps::gizmo`, a grid step and the ground of boxes. Its edits and its Undo go through a session of the pack, written when the project is saved, and its steps are its own stack of operations and their inverses, in memory (`src/state/workbench/maps.rs`). The Scene also serves a scanned GTA VI map body (`src/state/gta6_map.rs`) | Boxes, no model; one set of a project; the ground is the highest box, not collision |
| A whole map streamed and drawn | The rules of `crates/domain/or-world/src/streaming.rs`, the instanced draws of `crates/runtime/or-renderer/src/world.rs`, the fly camera of `crates/runtime/or-renderer/src/fly.rs`, the budgets and the occluder buffer beside `src/state/gta6_worldview/`. `crates/domain/or-depends/src/world_forest.rs` streams a GTA V install's maps for a view and selects their entities by those rules, with no screen (`openrage-cli world select`). For GTA VI also the install's day, the sun's shadow in cascades (`crates/runtime/or-renderer/src/world/shadow.rs`), the game's lighting passes, lamps, a sky and water ([the day](../features/GTA_VI_World_Viewer.md#the-day)) | Drawn for a GTA VI install only: nothing feeds the renderer the models of GTA V. World is a workspace on the rail while a GTA VI folder is set, with one viewer for the app (`src/state/gta6_worldview/mod.rs`) |
| The game's picture for GTA V | The Shaded viewport draws one model with GTA V's own passes, the install's timecycle, its sky and its water pass | One shadow tile, the sun alone. Nothing feeds GTA V's day to the world or draws its water: `crates/io/or-io/src/gta5/data/water.rs` reads and saves a `water.xml`, held on documents written by hand and on the water files of two installs |
| Collision | `crates/domain/or-world/src/collision/` answers a ray, a swept sphere, a box overlap and the ground over the bounds its caller hands it; `BvhCollider` in `crates/runtime/or-simulation/src/collision.rs` casts a ray at one bound, for particles; `crates/io/or-io/src/gta5/bounds/build.rs` builds and rebuilds a bound; `crates/domain/or-source/src/bounds.rs` holds a changed bound as a source over the game's file. `crates/domain/or-depends/src/world_collision.rs` hands the query the static bounds of a mount's column under a budget of bytes, behind `openrage-cli world ground`. `crates/domain/or-world/src/doors/` finds the doors of a set of maps and opens each through its arc against the query and the boxes near it, behind `openrage-cli maps doors`, of a project's map sets or of one map file of a mounted install. `crates/domain/or-maps/src/collision.rs` proposes the polygons under a placed object and takes them out, retypes or moves them (`openrage-cli maps collision`) | The collision of an interior and of a placed prop, which the entity's placement puts in the world; a screen that asks the query; a door's tuning; a screen that edits a polygon; the compare of a bound's source with its base |
| Roads, navmesh, scenario points | `.ynd` and `.ynv` owned and saved, the navmesh drawn, scenario regions typed; `crates/domain/or-source/src/paths.rs` holds a changed `.ynd` as a source, in a form that awaits the owner's word. `crates/domain/or-maps/src/roads/` edits the road graph of a folder of such sources by operations with inverses, each stating the set of documents it applies to, and holds its rules, behind `openrage-cli maps roads` | Roads and scenario points are not drawn, and no screen edits a road; a node disabled, which the file states no state for, and the lanes of a link, which its word does not name; an edit of a scenario point; the compare of a path region's source with its base |
| What a map holds besides entities | `crates/io/or-io/src/meta/typed/map_records.rs` types a map's parked vehicles, box occluders, occlude models, time cycle modifiers and both families of far lights as members of `CMapData`, as every RBM map of two installs states them, and a save refuses far lights out of step and an occlude model whose count misses its bytes; `crates/domain/or-world/src/map_data.rs` reads each as a view | A record in a map made from nothing, or in a file that holds none of its kind; an operation, a rule and a screen for any of them |
| Which file answers a name | `crates/domain/or-depends`: the type file of an archetype, what a map needs, the maps that place a model, a map's bounds by the manifest's groups and otherwise by filename (`crates/domain/or-depends/src/resolve.rs`). `crates/domain/or-depends/src/world_index.rs` answers the maps and the carried bounds at a position and in a column and is kept on disk, in a file of its own for a level of DLC that `openrage-cli world index`, `at`, `ground`, `find` and `select` mount at. `crates/io/or-io/src/cache_loader.rs` reads the game's cache files, which state each map's extents, each interior and each bound; no index uses them. The index keeps the gate of the group that requests a map, and `crates/domain/or-depends/src/find.rs` answers what a person typed (`openrage-cli world find`) | An index keyed by story or multiplayer, since a mode takes or leaves change sets and the index reads none; a mode for `world index` and `world at`, and a level for `maps doors --game` |
| The install as the game reads it | `crates/title/or-title/src/vfs/`: the mount, the mods overlay, a project's shadow folder; `crates/title/or-title/src/dlc/`: the DLC list and its change sets. The mount takes a level of DLC to stop at (`DlcCutoff` in `crates/title/or-title/src/vfs/game_mount.rs`; `--dlc` in `openrage-cli`, by pack or build number, and `DlcCutoff::Level`, by a pack's name in the DLC list) and the map as story or multiplayer (`MountOptions::mode`), both set by `openrage-cli dlc-list`, `maps edit-game` and the verbs of `world` over an install, of which `index` and `at` take the level alone | The app always mounts every pack and sets no mode; no server's resources as a layer |
| A project, its sources, its build | Content and Builds, a FiveM resource, a story pack, an `.oiv`, rules, History, links, the plan and its claims ([Project_Model_Design.md](Project_Model_Design.md)). A build packs a nested archive for any path through an archive's name, and a Maps content hands a story pack its maps inside one of the level's folder and each type file with its models inside one of the props folder, as the packs of two installs keep theirs (`crates/domain/or-maps/src/layout.rs`). `crates/domain/or-project/src/resource.rs` checks one resource folder as a server reads it, `crates/domain/or-project/src/server.rs` reads the resources of a server together and says what a build would write again, `crates/domain/or-project/src/place.rs` reads a zip as a place to adopt from, which `crates/domain/or-maps/src/mapping.rs` reports and adopts, `crates/domain/or-maps/src/changes.rs` says what a map changed of its base, `crates/domain/or-maps/src/put_back.rs` plans what puts one entity back as that base has it (`openrage-cli maps put-back`), and `crates/domain/or-maps/src/merge.rs` merges two copies of one map against the game's file (`openrage-cli maps merge`) | The story pack is locked for a content with a map, for the change set that turns its maps on, which no pack of either install shows for a pack that starts, and for a type file that declares an interior; no build asks the scan of a server, and no screen shows it, the report of a mapping, what a map changed, a plan that puts an entity back or a merge |
| Keys and places | A key is changed and kept in the preferences (`src/ui/shortcuts.rs`), one key for one command of a place (`Place` in `src/ui/commands/table.rs`); a row of the table can bind by where a key sits, and Settings offers presets of keys (`src/ui/shortcuts/presets.rs`). The GTA VI viewer lists places computed from its index and copies a camera line | A command bound by where its key sits, since no row is yet; the keys of CodeWalker's tools and camera in its preset, which holds one binding; a place a person saves |
| A ped that moves | The Animation workspace and the dressed ped of the Wardrobe | A cutscene played, a point of life, a person who walks |
| Proof that the game loads a file | No build says so ([held back](Workbench_Plan.md#held-back)). The sheet of what a person saw is `crates/domain/or-project/src/acceptance.json`, with its procedure in [In_Game_Acceptance.md](../guides/In_Game_Acceptance.md); the Build tab, `openrage-cli project build` and `wardrobe build` say which kinds of file of a build nobody saw the game load | Every result; a line for each kind a row writes; a second condition for a line an extension does not tell |

## How it fits the tree

What is proposed. [Where the tree stands](#where-the-tree-stands) says what exists, and here a sentence about the future names the row that makes it true.

### One world, two doors

Today World is a workspace and not a tab: its room is on the rail (`src/ui/panels/rail.rs`) while a GTA VI folder is set, and the app holds one viewer. [R70c](#the-gta-v-world-streamed-and-drawn-r70c) puts the room on the rail for a GTA V install too, where it opens a tab on the install's map, read only, with no project needed ([Open, Import and Add](Workbench_UX_Design.md#open-import-and-add)). R70c also makes the viewer one for each world tab, keyed by its stack, so an install's tab and a project's tab draw two stacks in one session.

With a GTA VI folder and a GTA V install both set, the room opens the world it showed last, on a first visit the install in use, and the first part of the stack line names the game and changes it. The Outline, Layers and Details of this plan are the panels of a GTA V world; a GTA VI world keeps its World panel ([GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md)).

The Map content of a project opens the same screen once R70c merges. What differs is the stack it draws and that its tools write. For a GTA V map the Scene panel is then the world viewport, where an object draws as a box while its model loads or when no file answers its archetype, the second with an outline of its own and a finding in Problems. A scanned GTA VI map body keeps the box scene, and the Plan panel stays.

With a project open the World tab stays read only. A tool that would write there says so in one line and offers Edit in the project, which brings the Map tab of a content forward at the same camera with the same object selected, and lists the contents when the project holds several ([R70i](#selecting-and-moving-in-the-world-r70i)). An edit in a Map tab goes to that tab's content.

### What a world shows

A world is a stack, read from the bottom:

1. The install, by the edition its files state, up to a level of DLC the person may choose, as story or as multiplayer.
2. Optionally a server: the resources of a folder the person names, read only, in the order the server starts them.
3. The content the project links, which is how a second project is seen in this world: drawn here, read into this project's builds, edited in its own window.
4. The project, last, as its build would ship it.

Each layer answers a name the way the game does, the newest winning, as the mount of `or-title` does today for the install, the mods overlay and a project's shadow folder. The level of DLC exists as the mount's cutoff: [R70b](#the-gta-v-world-index-r70b) passes it from the screen and adds the choice of story or multiplayer. [R70r](#other-resources-of-the-server-r70r) builds the server layer, and until it merges the stack has none. R70c draws the stack as a line of the screen's header, because what a mapper sees must be what the server will load.

The install shown and the target of the build are two things, and a map is where they must agree. A build moves a model and a texture to its output's target and ships a cooked map or type file as it is ([what the targets build](Project_Model_Design.md#what-the-targets-build)), while the members the world family states differ by title (R32). So a world shown from an install of one edition still builds for the other, its models and textures converted. A map taken from the other title's install is not: the header line says the mismatch, [R70h](#one-vocabulary-of-world-edits-r70h) refuses the first edit of a game object with a sentence that names an install of the target's edition, and it adds the rule that names a map source whose base is the other title's.

### How a project holds a change to the game's map

A project holds sources and no compiled game file. A map the project owns is a source. A map of the game that the person changes becomes one too, of the form a metadata source has today: its XML, and beside it the game's file as it came, which also compiles the XML ([metadata](Source_Format_Design.md#metadata)).

- **The first edit imports.** With a project open, moving or deleting an object the game placed imports that object's map and applies the edit as one step of History (R70h, with its line and its toast in R70i). With no project the tool offers to start one. This is a new door into a project, so it is [ruling 3](#rulings-asked-of-the-owner).
- **What ships is what changed.** A source still equal to the game's file is left out of a build, as a build does today.
- **The kept file is the base.** What changed is a comparison of two files, which R70y lists. R70r merges another resource's copy three ways and, when a title update changes the game's file under a source, raises a finding with the merge that carries the changes across.
- **A repository leaves it out by default.** A changed game map is a source taken from the game: the project's managed block of `.gitignore` names it, Check before sharing lists it and `openrage-cli project check-sharing` exits 1 on it ([commit policy](Source_Format_Design.md#commit-policy)). A clone opens without its changed maps until the project's owner turns on Commit sources taken from the game in Properties. Whether that stands is [ruling 17](#rulings-asked-of-the-owner); the toast of a first edit and the guide say it.
- **Some maps cannot become a source.** Only a file whose XML reaches its fixed point becomes one; R70h refuses by name the first edit of an object in a map that does not.
- **New objects go to a map of the project** (R70h), named in the header as the map that receives. A map past what one stream should hold is split at the build ([R70q](#a-build-a-server-and-a-story-install-take-r70q)): `crates/domain/or-world/src/partition.rs` plans a split by level of detail and applies nothing, and a split by area is new.
- **An object of an interior is in a type file.** It is an entry of the interior's archetype, in the interior's frame, attached to a room. Its first edit imports the type file that declares the interior, and a new object placed inside joins that archetype and the room under the pointer. Until [R70m](#interiors-r70m) merges, a tool refuses an object of an interior.
- **A bound and a path region have no source form.** An import keeps a `.ybn` and a `.ynd` whole ([how an import writes](Source_Format_Design.md#how-an-import-writes)). [R70w](#a-changed-bound-and-a-changed-path-region-as-sources-r70w) builds the form the rows that change collision and roads write to.

### One vocabulary of edits

Once R70h merges, every tool of the world calls a named operation of `or-maps`: place, move, delete with its chain, set a field, add a room, cut a polygon. An operation says what it reads and writes, applies to the sources, and returns its inverse. The operations grow from `or_maps::edit`, which renumbers links inside one file, and from the working set of `or-world`, which follows them across files.

- **Undo and Redo become steps of the project's History.** The Map screen's Undo is today a stack of its own, in memory, each step an operation and its inverse, refused when the source's XML is not the one the inverse states. History keeps a bounded count of steps (`KEPT` in `crates/domain/or-project/src/snapshot.rs`), each the project file read back whole, so no step depends on another. A world step holds its operation and the inverse, never two copies of a source: going back over several steps applies their inverses newest first, and a step whose source changed outside History refuses the whole move, as History does today for a source changed since its import.
- R70h adds `openrage-cli maps apply`, which takes the same operations from a file or from arguments, and the MCP server lists it as it lists every verb.
- A command of the screen offers Copy as command where its operation has a verb ([R69i](Roadmap.md#r69i-the-automate-room)).
- The live session of [R70u](#working-together-r70u) sends them.

An operation writes its source when it is made, and the source of a large game map is a large XML. R70h measures one drag on the largest map of the install, and what History keeps for a run of edits, before the tools are built on it.

### Collision as a query

Dropping to the ground, placing on a surface, the clearance of a door, walking and the judgement of which polygons belong to a building ask what a ray hits, what a swept shape touches and where the ground is under a point. R70g answers once, in `or-world`, over bounds its caller hands it, and those primitives stand on no other row. The bounds near a position are found through the index of R70b by the caller that has the mount, `or-depends` for the CLI and the app's streamer for the screen, because `or-depends` depends on `or-world` and not the reverse. The particle collider becomes a caller, which gives `or-simulation` an edge to `or-world` that points down.

### Where the code goes

Each edge was read against the crate's `Cargo.toml` and [Crate_Reference.md](../reference/Crate_Reference.md#layers).

| What | Where | Why there |
|---|---|---|
| The records carried today: rooms, portals, entity sets, the time cycle modifiers of an interior, parked vehicles, timecycle zones, occluders, the far lights | `or-io`, in the typed world family, with the engine's names | D15 and D16. A typed metadata root is no member of `FORMATS`, so an install suite holds each declaration |
| A GTA V reader for `water.xml`, a reader of the install's vehicle colours, the authoring of a `.rel` item | `or-io` | Bytes only |
| The choice of story or multiplayer, a server as a layer | `or-title`, beside the cutoff it has | It knows where a file lives |
| The world index: the maps and the bounds at a position | `or-depends` | It has the mount, and depends on `or-world` |
| Collision queries over bounds handed in, the polygons that belong to a building, the edit of a room or an occluder, the fit of occluders, door geometry | `or-world` | Rules of the world over typed data. It reads no file and sees no mount |
| The operations, the rules, the scene, placement, the parse of a place and the forms a script takes, the comparison and the merge of a map | `or-maps` | The content kind. It depends on `or-world`, `or-project` and `or-source`, not on `or-depends`, so an operation across files is handed the maps it reaches |
| The source form of a bound and of a path region | `or-source`, beside the kinds of `crates/domain/or-source/src/game_import.rs` | Where every source form is |
| What a server's resources ship and claim | `or-project`, beside adoption and the claims | The plan's vocabulary; no map is opened |
| The motion of a walking character | `or-simulation`, over the collision query | The edge to `or-world` points down. The app plays the clip with `or-anim`, which stays apart from `or-simulation` ([runtime crates](../reference/Crate_Reference.md#runtime-crates)) |
| The session of several people: the order of the operations and the wire, as bytes it does not read | A new foundation crate that opens no socket; the app encodes the operations and owns the connection | A foundation crate cannot know the operations of `or-maps` |
| The streamer for GTA V, the layers, the tools, the panels, and the seam between GTA V and GTA VI | The app. The seam is declared beside `src/state/gta6_worldview/`, where GTA VI's index, materials, lights, sky and water are, and says what a title supplies | `or-renderer` opens no file and does not depend on `or-world` |

A new edge that points up is refused as ever: `or-maps` does not see the renderer, and the manipulator stays maths. Nothing reaches the network unless a person asks; the one client in the tree is behind `openrage-cli script download-natives`.

What the editor can do, the CLI can do: each row names its verbs, the writing ones as one-shot commands of `openrage-cli` and not routes of the server, which stays a reader ([CLI_Server_Protocol.md](../reference/CLI_Server_Protocol.md)). Plugins keep what they have: a validator and an emitter extend a map's build, and an overlay of lines draws in the world.

## The order it is built in

A row is one lane and usually one pull request, and ends on a check that failed once before it passed. Sizes are as [Workbench_Plan.md](Workbench_Plan.md#the-second-plan-from-the-audit-of-2026-10-08) uses them. A row starts when the rows it depends on have merged, and rows that share no file run together. A stage is a point at which the result can be told to a mapper, not a gate: R70e and R70i follow R70c inside stage 2.

R70m and R70o each open with a data part, the declarations in `or-io` with their install suite, which waits for no screen row and may start in stage 1 ([what a kind of content costs](../reference/Crate_Reference.md#what-a-kind-of-content-costs)), so that R70f reads those records typed. R70s, R70t and R70u land in the parts their briefs list, each on its own check. The screen rows edit the rail, the tabs, Details and the command table, as the open rows of [R69](Roadmap.md#r69-one-window-of-tabs-and-a-rail) do, and the two never run together on one file ([the waves](Workbench_Plan.md#the-waves)).

| Row | Content | Depends on | Size | Stage |
|---|---|---|---|---|
| R70a | What the game loads | Ruling 1 | M | 1 |
| R70b | The GTA V world index | | L | 1 |
| R70g | Collision as a query | Nothing for the primitives; R70b for the bounds near a position | L | 1 |
| R70h | One vocabulary of world edits | | L | 1 |
| R70w | A changed bound and a changed path region as sources | | L | 1 |
| R70c | The GTA V world streamed and drawn | R70b | XL | 2 |
| R70e | Find anything and go there | R70b; R70c to go there; R70g for a height | M | 2 |
| R70i | Selecting and moving in the world | R70c, R70g, R70h | L | 2 |
| R70q | A build a server and a story install take | R70h | L | 2 |
| R70d | The day, sky, water and shadows of the world | R70c | L | 3 |
| R70f | Layers of what the game hides | R70c, R70g; the data part of R70o for three layers | L | 3 |
| R70j | A library of props and what comes in | R70i; ruling 2 for the thumbnails; ruling 5 for the XML reader | L | 3 |
| R70k | Removing what the game placed | R70h, R70i | L | 3 |
| R70r | Other resources of the server | R70c, R70h, R70q | M | 3 |
| R70y | What a project changed and putting it back | R70h, R70i | M | 3 |
| R70l | Editing collision | R70f, R70i, R70k, R70w | L | 4 |
| R70m | Interiors | Nothing for the data part; R70f, R70i; ruling 2 for the builder | XL | 4 |
| R70n | Doors | R70g, R70i | M | 4 |
| R70o | Parked vehicles, zones, lights and occluders | Nothing for the data part; R70f, R70i, R70k, R70l | L | 4 |
| R70p | Roads, scenario points and the navmesh | R70f, R70i, R70w | L | 4 |
| R70s | The model workshop | R70i; ruling 13 for the prop from an image | XL | 5 |
| R70t | Peds, cutscenes, life and walking the map | R70c, R70g; ruling 2 for the walk | XL | 5 |
| R70u | Working together | R70h, R70r; rulings 6 and 17 | XL | 5 |
| R70v | The keys, settings and guide of a mapper | R70i | M | 5 |
| R70x | The audio of an interior | R70m | L | 5 |
| R70z | A browser of models and vehicles | R70e, R70j; ruling 2 for the thumbnails | L | 5 |

R70 has no letter left: a finding that fits no row opens the next number.

What a mapper can be told at each point:

- **After stage 1** the one thing on screen is the Build tab's line of what is not checked in game. The tree can say what is at a position, what a ray hits and what an edit is, and holds a changed bound as a source.
- **After stage 2** a person opens the map, finds a place, duplicates an object, swaps its model, drops it to the ground and builds a resource: the first thing to hand to a mapper.
- **After stage 3** a prop is chosen from a list, a deletion follows its chain across files, the layers show what the game hides, the project's changes are listed and put back, and the server's conflicts are found. The collision and the occluders of a deleted building still stay.
- **After stage 4** every kind of thing a map holds can be edited, the rooms and portals of an interior once ruling 2 is answered.

## Briefs

Each brief says what to read first, what to build, and the checks, each seen failing before it passed. The Roadmap row a brief's checks link to holds the Done when the row closes on, in the same words where both state a check. [What every wave does](Workbench_Plan.md#what-every-wave-does) applies: documents travel with the code, every sentence of a screen is translated, screens are checked on `just shots`, and a control joins [the control inventory](Workbench_UX_Design.md#control-inventory). A row that writes a kind of file adds it to the recipe and the sheet of R70a.

### What the game loads (R70a)

Read [what is held back](Workbench_Plan.md#held-back), the open clauses of [R62](Roadmap.md#r62-the-rbm-layout-synthesiser), [R51c](Roadmap.md#r51c-dlc-pack-creation) and [R33e](Roadmap.md#r33e-navmesh-index-stability), and [R32c](Roadmap.md#r32c-cache-files-metadatadat-and-the-audio-sector-grid) on the cache files.

Build the sheet, the test that reads its format, the Build tab's reading of it, and a guide page with the steps to load a build on a FiveM server and in story mode. A line of the sheet records the date, the game's build and a result, such as that the far model is gone, that an object moved past its map's old extents still streams, or that a second instance of an interior is entered. Build the sample as a recipe and never as a project in the repository: its sources are made on the person's machine over their own install, because a changed game map or bound holds the game's content, which no fixture and no commit takes. It starts with the kinds of file a build writes today; the lines that change a game file arrive with R70h.

Checks ([R70a](Roadmap.md#r70a-what-the-game-loads)): the sample builds headless over an install and every file reads back (slow tier, named in the skip ledger without an install); a kind of file a row writes and the recipe lacks fails a test; a test reads the sheet's format; the Build tab says "not checked in game" for a kind whose line is empty.

### The GTA V world index (R70b)

Read `crates/title/or-title/src/vfs/game_mount.rs` for the cutoff, `crates/domain/or-depends/src/resolve.rs`, `crates/io/or-io/src/cache_loader.rs`, `crates/domain/or-world/src/state.rs`, and `src/state/gta6_worldview/index_keep.rs`, which keeps the GTA VI viewer's scan.

Build the index of a mounted GTA V install: every map with its extents, parent and the group that requests it; every archetype with its type file and box; and two queries by position, the maps and the bounds. One bound overlaps many maps, so the bounds at a position come from the record the cache files keep of each bound, checked against the bounds themselves in the install suite. Pass the mount's cutoff from the screen, add the choice of story or multiplayer to `or-title`, and key the index by what the install holds, the cutoff and the choice. Keep it on disk and rebuild only what changed. Add `openrage-cli world at`, which answers a position, and `openrage-cli world index`, which builds and reports it.

Checks ([R70b](Roadmap.md#r70b-the-gta-v-world-index)): over a mounted install the index names every map and type file the mount lists, and a position inside a known map's extents answers that map (install suite, slow tier, named in the skip ledger without one); a second start opens no archive that did not change; an index kept at one cutoff is not read at another; story hides what multiplayer adds, on a synthetic mount.

### The GTA V world streamed and drawn (R70c)

Read [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md), `crates/domain/or-world/src/streaming.rs`, `src/state/gta6_worldview/mod.rs`, `src/ui/panels/rail.rs`, `src/ui/dock.rs` and `src/state/gta6_map.rs`.

Build in parts, each on its own check. First the seam between what the World screen takes from a title (where maps, models and textures come from, how a material is drawn, what feeds its day) and what it shares (the selection of levels, the budgets, the wait for textures, the occluder buffer, the camera). Then the viewer made one for each world tab. Then the GTA V feed from R70b: models, textures and materials for both targets, interiors and their objects, and the content a project links, drawn as its layer of the stack. Then the room, the tab and the Map content's Scene as [one world, two doors](#one-world-two-doors) has them.

Draw the stack as a bar under the tabs of a World tab and of a Map screen, which changes the rule that a screen of a project has no bar ([Switch view](Workbench_UX_Design.md#switch-view)); the bar says when the install shown is not of the build's edition. Give the World kind of tab its primary command in the table of [R69f](Roadmap.md#r69f-a-rail-of-rooms-and-one-primary-action-a-tab). The shared camera takes the inputs [World_Editor_UX_Design.md](World_Editor_UX_Design.md) states, for both titles. The viewer's settings gain presets for a modest and a strong machine and a frame cap.

Checks ([R70c](Roadmap.md#r70c-the-gta-v-world-streamed-and-drawn)): a synthetic world of parent and child levels draws the level the rules select at each distance, in real frames with no window; an install's tab and a project's tab draw two stacks in one session; the GTA VI screens and a scanned GTA VI map body draw the same frames as before the seam, and [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) states their new camera inputs; the French screens of the world hold at the smallest window; a frame over the city is measured on the owner's machine, its budget recorded.

### The day, sky, water and shadows of the world (R70d)

Read [Viewport_Shading_Modes.md](../features/Viewport_Shading_Modes.md), the drivers under `src/state/drivers/`, [the day](../features/GTA_VI_World_Viewer.md#the-day) of the GTA VI screen and `crates/runtime/or-renderer/src/world/shadow.rs`.

Build the feed of GTA V's timecycle, weather and water into the lighting, the shadow cascades, the sky and the water the World screen has for GTA VI, behind the seam of R70c, and say which part of each is GTA VI's alone and is written again. Declare a reader for GTA V's `water.xml` in `or-io` and draw the water it describes. The distance shadows reach is a setting. Measure what drawing GTA V's materials through the game's own shaders costs over the whole map, for ruling 9.

Checks ([R70d](Roadmap.md#r70d-the-day-sky-water-and-shadows-of-the-world)): the reader opens the water file of both installs and saves its fixed point; the same hour and weather give the same sun on a model in Shaded and in the world; a GTA VI day draws the same frames as before.

### Find anything and go there (R70e)

Read [R59j](Roadmap.md#r59j-one-command-registry) and [R24b](Roadmap.md#r24b-go-to-file), `src/ui/panels/command_palette.rs`, `src/ui/panels/hierarchy/navigation.rs`, which sends the key of Go to File to the field of Files while a file is open, and `src/ui/panels/world_viewer_places.rs`.

Build places in the palette: a position in the forms scripts write, a hash in each spelling, a model, a map, a type, in the box and under the field of Files, a position or a hash first; on a world tab the key of Go to File opens the box. A model's row carries its count of placements, goes to the nearest and steps through the rest. Add Show in world to a file's row, an archetype's row and a row of Uses and Placed by. Add bookmarks, places a person saves from the box, each with a key to go to it, and one to set it once R70i's binding honours Shift with a digit; Ctrl with a digit is the tabs. Add Copy for script in Lua, JSON and C#, as templates in data: a position, with or without its heading, a model's name, its hash in both spellings, a list of a selection, the lines that spawn an object or a vehicle; the button keeps the form used last. Two numbers take their height from the ground of R70g. Add `openrage-cli world find`.

Checks ([R70e](Roadmap.md#r70e-find-anything-and-go-there)): each form of a position parses to the same point; a hash typed three ways finds one model; a copy in each form parses back to the position and the heading it came from; two numbers over a known road go to the road and not to sea level; a model placed twice steps from one placement to the other; a pasted position is the first result with a file open.

### Layers of what the game hides (R70f)

Read `src/render/navmesh.rs`, `crates/domain/or-world/src/map_data.rs` for what is read and what is only counted, and [what streams](../features/GTA_VI_World_Viewer.md#time-weather-and-what-streams) on the World panel.

Build the layers, each drawn within its radius: road nodes and links by category, navmesh by category, scenario points with their heading, collision by material and by version over the bounds R70g keeps near the camera, rooms and portals, occluder boxes and occlusion meshes with a preview of what they hide, the levels of detail, and sound zones and fixed emitters, each sound played when selected. Parked vehicles, timecycle zones and far lights are drawn once the data part of R70o types them. Add the presets, the pick order, and the list of maps only a script requests: the World panel's section of script-managed groups, fed for GTA V from `crates/domain/or-world/src/state.rs`, with a stated naming rule for two versions of one place.

Checks ([R70f](Roadmap.md#r70f-layers-of-what-the-game-hides)): each layer draws a synthetic file's records at their places in real frames; a preset shows the layers it names and no other; a layer off costs no draw.

### Collision as a query (R70g)

Read `crates/runtime/or-simulation/src/collision.rs`, `crates/io/or-io/src/gta5/bounds/build.rs` and `crates/domain/or-depends/src/resolve.rs`.

Build in `or-world`, over declared bounds handed in, a ray, a swept sphere and a box overlap, with the hit's material, and the ground under a point that ignores walls. These stand on no other row. Then build the set of bounds near a position, read through the index of R70b and kept by the caller that has the mount. Make the particle collider call the query. Add `openrage-cli world ground`, which answers a position.

Checks ([R70g](Roadmap.md#r70g-collision-as-a-query)): each primitive of a bound is hit where arithmetic says, on bounds built by hand; over an install, the ground under a position on a known road is within a step of the road's nodes there (slow tier); the particle preview's collisions are unchanged.

### One vocabulary of world edits (R70h)

Read `crates/domain/or-maps/src/edit.rs`, the header of `crates/domain/or-world/src/execute.rs`, `crates/domain/or-world/src/generate.rs`, [what project history builds](Workbench_UX_Design.md#what-project-history-builds), the Undo of [the Map screen](Workbench_UX_Design.md#what-the-map-screen-builds) and the open clause of R62.

Build the operations, each with its inverse and a form as data; an inverse carries the extents its operation replaced. Carry the working set's renumbering across files, for a create, a delete and a delete with its chain, over the typed roots, so that it saves with no schema snapshot. Add `openrage-cli maps apply`.

Build the first edit that imports a game map as a source with its base. It refuses by name a map whose XML does not reach its fixed point and a map of an install of the other edition than the build's target, and a rule names a source whose base is the other title's. Its step records the source's digest after the edit, so that a move back over it is not refused, and its toast offers Undo of the step.

Build a new map and a new entity from a donor of the mounted install, in the donor's container and with its member set, until a build from empty is sound: nothing in the tree derives a class's signature word (R62, open), the members of an entity differ by title, and `generate.rs` refuses a donor that would leave a zero signature. With no install, the run refuses by name.

Move the Map screen's stack into History as [one vocabulary of edits](#one-vocabulary-of-edits) has it, the part of [R69h](Roadmap.md#r69h-one-undo-for-the-tab-on-screen) a Map tab needs below the screen, and make the measures that section names.

Checks ([R70h](Roadmap.md#r70h-one-vocabulary-of-world-edits)): every operation followed by its inverse leaves the sources byte for byte, and a test fails when an operation has no inverse; a list applied by the CLI and by the screen gives the same sources; a deletion with its chain runs under `OPENRAGE_SCHEMA=none`; a new map reads back as its root, passes the rules and carries no zero signature; an edit is taken back after the project is opened again; History goes back over a first edit that imported; a first edit over the other edition's install writes nothing, and a rule names a source whose base is the other title's; the install suite counts the maps a first edit refuses.

### Selecting and moving in the world (R70i)

Read `crates/domain/or-maps/src/gizmo.rs`, [what the Map screen builds](Workbench_UX_Design.md#what-the-map-screen-builds), `src/ui/commands/target.rs`, `src/ui/shortcuts.rs`, and [R69g](Roadmap.md#r69g-one-details-panel) and R69h, whose Details panel and one Undo the row draws in.

Build the pick by pixel, through an id pass, with its second click; the rectangle that takes what is visible; the group move; the choice of axes; copy, paste and duplicate, with the duplicate that repeats the last offset; the snap to another object's box; the swap of a model; the drop to the ground and the follow, over R70g. Draw the manipulator in the world. Build the line that offers a project, the toast of the first edit, Edit in the project from a read-only World tab, the hints of each tool in the status bar, the menu of a right click on an object, and the double click that opens its model.

The tools, the drop to the ground and the turn of a ghost are rows of the command table, the tools bound by where a key sits, a binding that honours Shift with a digit, since the matcher reads a press by its character first. Delete, Copy and Paste gain a target for placed things in `src/ui/commands/target.rs`, and Remove entity, Stand on the ground and Add an entity are the Map screen's commands, given keys. The view reads Escape only while it holds the focus, since the palette reads that key without taking it. The rule `guid-twice` gains its safe fix, a new guid for each copy.

Delete removes an object of the project. Until R70k merges, Delete on an object the game placed is refused, saying its far models and its collision would stay; until R70m merges an object of an interior is refused. A game object that moves takes the levels of its chain that stand for it alone, each file named before the step; a far model shared with a neighbour stays and is named.

Checks ([R70i](Roadmap.md#r70i-selecting-and-moving-in-the-world)): `the_maps_screen_on_screen` extended to a drag in the world on a model, as one step of Undo; a pick through a hole in a fence takes what is behind it, and a wall stops it; a dropped object lands on the collision under it, not on a roof above; a repeated duplicate leaves equal spacing; Delete on a game object writes nothing and says why; a moved object's far model follows it, and a shared one is left and reported; Escape that closes the palette keeps the selection; a tool's key answers by its position on an AZERTY layout; each tool and a drag show their hints; an object's menu draws from the registry (`every_context_menu_draws_from_the_registry`); a double click opens an object's model; Fix all safe leaves every guid stated once.

### A library of props and what comes in (R70j)

Read the links and adoption of [Project_Model_Design.md](Project_Model_Design.md#what-links-adoption-and-templates-build), `src/render/thumbnails.rs` and `src/ui/panels/import_report.rs`.

Build the Library tab as a list by name, with a search, over linked projects and the game's props by a category derived from the install. Build the ghost and the placement by surface, a zip as a place to adopt from, the recognition of files an asset escrow locks, kept whole and drawn as a box, and the map rules in the import report. Two parts wait: the thumbnails for ruling 2, and for ruling 5 the reader of the XML a Blender export writes and the watched folder.

Checks ([R70j](Roadmap.md#r70j-a-library-of-props-and-what-comes-in)): the Library lists a linked project's props and a synthetic install's by derived category; a ghost lands on the surface under the pointer, on bounds built by hand; a prop dragged from a linked project builds with only the files it needs; a locked file survives a build byte for byte and is named; a zip that reaches outside its folder is refused.

### Removing what the game placed (R70k)

Read `crates/domain/or-world/src/group.rs` for `MapGroup`, and `delete_subtree` in `crates/domain/or-world/src/execute.rs`.

Build the deletion's preview and its operation over R70h: the chain across files, the parents' counts, each file touched named before the step. Add the copy of the script line that hides a model, the flag for objects that cannot be told from a twin, which are left, and the rule for a door left where a building was deleted. The building's collision is R70l's and its occluders R70o's: until they land, the preview says both stay.

Checks ([R70k](Roadmap.md#r70k-removing-what-the-game-placed)): over an install, deleting any object with its chain leaves every link and count valid, as `executing_a_plan_keeps_every_link` holds for a map; undo restores every file; twins are flagged and left; a door inside the box of a deleted building is reported.

### Editing collision (R70l)

Read `crates/io/or-io/src/gta5/bounds/`, [R41c](Roadmap.md#r41c-bounds-and-physics-construction) on what a rebuild permutes, and R70w.

Build the collision tool: pick polygons, a rectangle that takes the visible ones, delete, set a material from the game's list, move with shared corners following. Write the three versions of an area together, to the source of R70w. Build in `or-world`, over R70g, the association of a building with its polygons. From it build the proposal of polygons to remove with a deleted building, as data the person edits before it applies, which leaves ground slabs and decals unless an option, off by default, takes them; and the option that moves a building's collision with it, which takes the polygons inside its box, leaves the ground and says what it moved.

Checks ([R70l](Roadmap.md#r70l-editing-collision)): an edited bound passes the validation that refuses what the game would misread, and reads back with the polygons expected; the three versions stay alike in what was edited; on bounds built by hand the association takes a building's walls and leaves the ground, and with the option on takes the slab too; undo restores every file.

### Interiors (R70m)

Read [what wave P44 built for Maps](Workbench_Views_Design.md#what-wave-p44-built-for-maps) for the members the install states, [what a kind of content costs](../reference/Crate_Reference.md#what-a-kind-of-content-costs) and `crates/domain/or-maps/src/rules.rs`.

Build the data part first: rooms, portals, entity sets and the interior's time cycle modifiers declared in the typed world family of `or-io`, with the engine's names ([ruling 14](#rulings-asked-of-the-owner)).

Build what needs no builder: the entity set switches and their script lines, a set whose name the index lacks written as a comment; placement inside an interior, as [an object of an interior](#how-a-project-holds-a-change-to-the-games-map) has it; and the sound passages and the room audio shown read only, with a notice on a game interior that was moved or copied, once how the game finds an interior's occlusion file is measured.

Build the health rules: a portal that names a room the interior lacks, has not four corners, has no area or is not flat; two portals alike; an object in no room or in a room the interior lacks; a model nothing holds; a room with no portal; an interior with no portal to the outside; a room whose box is empty or far larger than its objects; an entity set empty or named twice; more rooms than the room number of a collision polygon holds.

The builder waits for ruling 2: the edit of a room's box, name and settings, the portals on a moved wall following; of a portal's corners, rooms and flags, fitted to the nearest wall with its corners in the order the game expects; an object attached to a portal or filed in an entity set; an entity set made and renamed; a room or a portal added and removed; a new interior, a copy under a new name and a second instance.

Checks ([R70m](Roadmap.md#r70m-interiors)): the declared members are the ones the install states, held by the install suite; every interior of an install reads typed and saves to its fixed point; each health rule fires on an interior built to break it; a prop placed in a room builds into the type file and into no map.

### Doors (R70n)

Read `crates/domain/or-world/src/extension.rs`, which types the door extension, and R70g.

Build the search of doors, first by the door extension an archetype carries, counted over an install, then by model name, saying how many models had no name. Build the pairing; the copy forms as templates, among them the door tuning entry and the line that gives it to a model; the arc and the clearance over R70g; a door that opens in the view and stops where its arc is blocked; the zone in which a door opens by itself, drawn, with its tuning read; and the fitted replacement. Build the door rules: a door model of the project whose archetype lacks what the game's doors carry, measured on the install first; a game door moved or deleted, since a lock script finds a door by model and position; two doors at one place; a door whose arc is blocked.

Checks ([R70n](Roadmap.md#r70n-doors)): two leaves that face each other pair and two that do not stay single; a door behind furniture is reported blocked and the same door in an empty room is not; each door rule fires on a map built to break it.

### Parked vehicles, zones, lights and occluders (R70o)

Read `crates/domain/or-world/src/map_data.rs` and [what a kind of content costs](../reference/Crate_Reference.md#what-a-kind-of-content-costs).

Build the data part first: parked vehicles, timecycle zones, far lights and occluders declared in the typed world family of `or-io`. Then build their place, move, size and fields, the far lights' two maps kept in step, and occlusion meshes drawn and removed as boxes are. Build the occluder fit as a proposal, which for an interior puts thin boxes in its outer walls and leaves its portals to the outside free, and from the association of R70l the proposal of the occluders over a deleted building. Build the occluder rules: a box that reaches outside its building, that a road or a pavement crosses, that overlaps an interior, that is too small to hide anything, or that lies mostly inside another. Draw a game vehicle on a selected parking spot, and copy a zone's centre for a script.

Checks ([R70o](Roadmap.md#r70o-parked-vehicles-zones-lights-and-occluders)): the declared members are the ones the install states, held by the install suite; every record of an install reads typed and saves to its fixed point; the far lights' two maps agree after an edit; a fitted box lies inside the collision it was fitted to, on a building built by hand; each occluder rule fires on a box built to break it.

### Roads, scenario points and the navmesh (R70p)

Read `crates/io/or-io/src/gta5/path_region.rs`, where a link names its far node by region and index, `crates/domain/or-world/src/scenario.rs`, the risk of [R33](Roadmap.md#r33-paths-navmesh-scenarios-and-tracks) and the form of R70w.

Build the edit of a road node's fields and lanes, and nodes and links added and removed. Removing a node renumbers its region, so every region that names a renumbered node is written in the same step and named before it, and a node another region names is disabled and not deleted. Build the edit of a scenario point, in regions that exist, with its chain following. Build the rules: a vehicle node with no link, a link to a missing node, a link the other node does not return, a vehicle link with no lane, a road node linked to a footpath node, a node or a scenario point above the ground or under a surface, a scenario point left on a deleted object, a building added on walkable ground, and a deleted building whose floors stay in the navmesh. No navmesh is authored.

Checks ([R70p](Roadmap.md#r70p-roads-scenario-points-and-the-navmesh)): an edited region and every region that names one of its nodes read back with every link resolving; a deleted node leaves no link to it in any region; a node another region names is disabled and not deleted; each rule fires on a file built to break it.

### A build a server and a story install take (R70q)

Read [Dlc_Pack_Creation.md](../features/Dlc_Pack_Creation.md#what-is-verified), [what the targets build](Project_Model_Design.md#what-the-targets-build), `crates/domain/or-world/src/manifest.rs` and `crates/domain/or-world/src/partition.rs`.

Build, in a map's contribution, the manifest of its maps and the line that declares the resource a map, and show after a build the line for the server's configuration. For a story pack, hand the maps and the type files over at a path through an archive's name, which the build already packs as a nested archive, registers and counts (`a_path_through_an_archives_name_is_packed_as_that_nested_archive`). The row adds that path in the Map content, the contents the install's packs state when they register an archive of maps, and a type file named inside that archive; then the cell unlocks ([ruling 15](#rulings-asked-of-the-owner)). Split a map past its extents at the build: `partition.rs` plans the split by level of detail and nothing applies it, and the split by area and the links carried across the parts are new.

Checks ([R70q](Roadmap.md#r70q-a-build-a-server-and-a-story-install-take)): a project's build writes a `_manifest.ymf` that reads back and names what the project's maps place, seen failing on a build that writes none; a changed game map whose derived lists differ from the shipped ones keeps the shipped entry; a built pack lists its maps where the install's own packs list theirs; the cell says yes only with the archive written; every link of a split map resolves; two builds are the same bytes, and one added object changes only the part that holds it.

### Other resources of the server (R70r)

Read `crates/domain/or-project/src/adopt.rs`, the plan's claims, the overlay of `crates/title/or-title/src/vfs/` and the comparison of [R49e](Roadmap.md#r49e-compare-a-file-with-another-or-with-the-original).

Build the scan of a folder of built resources into what each ships and claims, a server's or the builds of a person's other projects. Mount a server's folder as the read-only layer of [the stack](#what-a-world-shows), in `or-title`. Build the finding that names two resources shipping one file as a rule of the Problems panel, raised by the edit that imports such a file and again before a build writes; the first edit of a map the server layer overrides imports the game's file as the base and offers the resource's copy as changes to merge.

Build the three-way merge of a map with the game's file as base, reading what two copies differ in by record and field. A bound, a road file or an occlusion map that two resources ship is named and not merged, until R70w gives that merge a form. When the index is rebuilt, a source whose base is no longer the install's file is a finding, with the merge that carries its changes across.

Add `openrage-cli project check-server` and `openrage-cli project check-resource`, the second for any folder. Its findings, each named: no manifest; no version line; maps and no line that declares the resource a map; a file the manifest or a data line names and the folder lacks; one file name twice under the stream folders; a file of a kind the server does not stream; a file whose streaming memory passes the size at which the server warns, cited where it was read; a file that does not read or that an asset escrow locks; and, from the map rules, `archetype-missing`, `archetype-without-model`, an archetype whose texture dictionary nothing holds, and `model-unused`. A build warns when its folder lies under a folder of the server's resources that is not a category, and names an earlier build of the same resource elsewhere.

Checks ([R70r](Roadmap.md#r70r-other-resources-of-the-server)): two resources that change different objects of one map merge with no conflict, and two that move the same object report it; two that ship one bound are named and not merged; each finding of the resource check fires on a resource built to break it, and a sound one passes; a synthetic resources folder that ships one map draws it in place of the install's; importing a map another resource ships raises the finding before any build; a build folder under a folder that is not a category is warned about.

### The model workshop (R70s)

Read [Source_Format_Design.md](Source_Format_Design.md) and [Mesh_Interchange.md](../features/Mesh_Interchange.md).

Build in parts, each on its own check. First, as operations on a model source: list its parts, each hidden or shown alone; remove a connected part in every level; move and copy a part; add a part copied from another model; add a box or a plane with a material of the model; draw the neighbouring objects around it on request. Second, since an edit of a game model changes every placement: say how many placements use it, and offer Save as a new prop, the model under a new name with a type file that holds a copy of its archetype, the entity the workshop was opened from swapped to it. Third, after ruling 13, a prop made from an image. Last and marked experimental, an opening cut into a wall.

Checks ([R70s](Roadmap.md#r70s-the-model-workshop)): a model with a part removed recooks, reads back and draws, the part gone from every level; after Save as a new prop the game's archetype still resolves to the game's file and one entity names the new one.

### Peds, cutscenes, life and walking the map (R70t)

Read [Animation_Preview.md](../features/Animation_Preview.md) and R70g.

Build in parts, each on its own check. First a browser of peds with outfits and clips over the Animation workspace, which copies a ped's model name and the lines that play the clip shown. Then a player of a cutscene's tracks in the world, with no sound. Then what a scenario point would spawn at the hour shown. Last, after ruling 2, a character that walks, climbs and swims over the collision query, labelled as a check of the path and not the game's own movement.

Checks ([R70t](Roadmap.md#r70t-peds-cutscenes-life-and-walking-the-map)): the copied lines name the clip shown; a cutscene's cuts fall at the times its file states; a point lists what its type and model set allow at the hour shown, on a region built by hand; the character stops at a wall and steps a kerb on bounds built by hand.

### Working together (R70u)

Read [what project history builds](Workbench_UX_Design.md#what-project-history-builds), `crates/domain/or-source/src/git.rs` and the merge of R70r.

Build three parts, each usable alone. First, the merge of R70r wrapped as the driver Git calls for a map source, so two people who share a repository merge by object and not by line; it merges a changed game map only where the repository holds the sources taken from the game (ruling 17). Second, a session on a direct connection: the host orders the operations of R70h and sends them, with each person's selection and camera. What a person holds selected is locked to the others, and a tie goes to one of them with a line that says so. The host admits each person, a guest whose stack differs from the host's is refused with what differs, only the host builds, and a person can go to another's view. A person's Undo takes back their own last step, carried out by the host; a guest who lost the session joins again; and the host sets how many may join. Third, with ruling 6, a relay that forwards bytes it cannot read.

Checks ([R70u](Roadmap.md#r70u-working-together)): the driver merges two branches that change different objects of one map; two edits that cross are ordered the same for every person; a person who joins late has the same sources byte for byte; an operation on a thing another person holds is refused and names who holds it; a guest with a different stack is refused with what differs; a person's Undo leaves another's step in place.

### The keys, settings and guide of a mapper (R70v)

Read `src/ui/shortcuts.rs`, `src/state/front_door/template.rs`, [Workbench_Guide.md](../guides/Workbench_Guide.md) and [Interface_Languages.md](../guides/Interface_Languages.md).

Build presets of keys, with one that matches CodeWalker's, and the place of a binding in `src/ui/shortcuts.rs`, where a conflict is today one key for two commands of the whole table. Build the settings a mapper changes on one page: mouse sensitivity, field of view, an inverted vertical axis, the movement letters of the keyboard's layout, the ground grid, the snap steps and the layer radii, with Restore defaults. Give a panel's title a sentence on its game term as its hover. Wrap the sentences of the map rules into the catalogs, which a library crate's sentences are not today. Build a tour of the world, a chapter of the guide and a Map template that opens on the world.

Checks ([R70v](Roadmap.md#r70v-the-keys-settings-and-guide-of-a-mapper)): no preset binds one key to two commands of one place; every step of the tour points at a control that is drawn; each setting of the page changes what it names in a frame drawn with no window; a finding of a map rule draws in French.

### A changed bound and a changed path region as sources (R70w)

Read [the layout per format](Source_Format_Design.md#layout-per-format), which leaves the source of a `.ybn` for later and has no row for a `.ynd`, [how an import writes](Source_Format_Design.md#how-an-import-writes) and `crates/domain/or-source/src/game_import.rs`.

Build the form in which a project holds a bound and a path region it changed: the source, the base it was taken from, its cook and its compare against that base. A project holds no compiled game file, so R70l and R70p write to this form. The row draws nothing.

Checks ([R70w](Roadmap.md#r70w-a-changed-bound-and-a-changed-path-region-as-sources)): every `.ybn` and `.ynd` of an install becomes a source and cooks back to the canonical save of its file (slow tier, named in the skip ledger without an install); an untouched source is left out of a build; a source with one polygon removed compares to its base as that polygon.

### The audio of an interior (R70x)

Read [R34b](Roadmap.md#r34b-the-audio-metadata-schemas), under which a `.rel` item reads typed and no typed field is edited, and [metadata](Source_Format_Design.md#metadata), which spells the members of the occlusion metadata as hashes.

Build first, in `or-io`, the authoring of the game data items of an interior and its rooms in a `.rel`, with a save that reaches a fixed point. Then write the occlusion file from the rooms and portals of R70m, through a donor's type table; a typed root for it waits for ruling 16. Then add the audio lines to the resource's manifest.

Checks ([R70x](Roadmap.md#r70x-the-audio-of-an-interior)): a `.rel` with an item added reads back typed and saves to its fixed point; the occlusion file of an install's interior, written again from its rooms and portals, reads back through the reader that opens the game's; a resource with an interior registers both files.

### What a project changed and putting it back (R70y)

Read [metadata](Source_Format_Design.md#metadata) for the base a source keeps, the comparison of R49e, and [the edit loop](Source_Format_Design.md#the-edit-loop), where the check against the game runs only when asked of one source.

Build the comparison of a map source with its base as data: entities added, moved, deleted and changed in a field, and those that cannot be told from a twin, listed as undecided and never as deleted. Add `openrage-cli maps changes`, which prints it. Build the panel that lists it, with counts, a filter, and a click that selects the object or goes to where a deleted one stood; a deleted object draws in the world as an outline while its row is selected. Build the operation that puts one object or one deleted building back as the base has it, with the far models a deletion took, as one step, and Discard changes to this map after a question that counts them.

Checks ([R70y](Roadmap.md#r70y-what-a-project-changed-and-putting-it-back)): a source whose map moves one entity, deletes one and adds one compares to exactly those three, and two twins swapped compare as undecided; the panel lists the three, and a click on the deleted one goes to where it stood; putting each back leaves the source equal to its base.

### A browser of models and vehicles (R70z)

Read `src/render/thumbnails.rs` and the Library of R70j. Nothing in the tree interprets the install's vehicle colours and variations.

Build the browser of every object type and every vehicle of the game: filters by class, by pack and by size, favourites, the models opened last, back and forward; where the model shown is placed, nearest first; and a command that saves the view of a model as a PNG, with a transparent or a keying background, a size and preset views. For a vehicle, read the install's vehicle colours and variations in `or-io`, draw it in the combinations the game plans, copy the lines that paint it in the colours shown, and show its sheet. The grid of thumbnails waits for ruling 2.

Checks ([R70z](Roadmap.md#r70z-a-browser-of-models-and-vehicles)): a filter by class lists the vehicles a synthetic install's data puts in that class and no other; a saved picture has the size asked and is transparent where nothing is drawn; a synthetic install's vehicle draws in each combination its data lists.

## Rulings asked of the owner

Each has a default. A row takes it until a sentence reverses it, except a default that reopens a dated owner decision or builds what [Workbench_Plan.md](Workbench_Plan.md#held-back) holds back: there the ruling says "It waits", and that part is not built before the owner answers.

1. **Who loads a build in the game.** Default: the owner, from the sheet of R70a, once a stage. No build says that it loads in game, which stays [held back](Workbench_Plan.md#held-back).
2. **The interior builder, the walk mode and the thumbnail browser are built in the app**, where R47 files them as plugins. Recommended: in the app, since a plugin cannot draw a mesh. It waits: the builder of R70m, the walk of R70t and the thumbnails of R70j and R70z are built only after the answer.
3. **The first edit of a game object imports its map into the project** without a question, as one step with its toast. Default: yes.
4. **A server's resources folder is a place OpenRage reads**, and a build's folder may be set inside it, where a marker protects every folder the build did not write. Default: yes, remembered for the machine and never in the project file.
5. **The XML a Blender export writes is read on import**, for drawables, textures, bounds, type files and maps, landing as sources, and is never written: NodeRunner also writes it from a game file, and here a model leaves as glTF ([Mesh_Interchange.md](../features/Mesh_Interchange.md)). Recommended: yes; without it a prop arrives as glTF, with no collision and no type file. It waits: this reopens the ruling of 2026-09-22 and is held back, so R70j builds its reader and the watched folder only after the answer.
6. **A live session reaches the network when a person starts one**, and a relay is a program a team runs for itself. Default: yes to both; OpenRage runs no service.
7. **A story pack is installed into the mods folder from the build.** Default: no; the build writes the pack and states the line the DLC list needs. It waits: the entry of 2026-11-21 left installing into the game undecided, and a yes brings the install, a list of the packs installed, and their removal.
8. **The tools are on the keys 1 to 5, and the camera flies on the right button or on a toggle key**, in place of the drag that turns the GTA VI viewer and the drag that orbits the Map scene. Default: yes, with a preset that matches CodeWalker's keys, an option the entry of 2026-10-03 left to the owner.
9. **The materials of the whole map are drawn through the game's own shaders**, as one model's are in Shaded; the World screen's lighting already goes through the game's passes. Default: not in the first cut; R70d measures it.
10. **Forms for other projects' door scripts are kept as templates in data.** Default: yes.
11. **The game's props are grouped by rules over the install**, and no list of names is shipped. Default: yes.
12. **What [World_Editor_Parity.md](World_Editor_Parity.md#left-out-and-why) lists is left out**, among it the earlier games, an updater and a panel as a window of its own. Default: yes.
13. **A drawable is made from nothing**, which a prop made from an image needs and [Mesh_Interchange.md](../features/Mesh_Interchange.md#what-this-does-not-do) lists as not done. It waits: author mode is held back, and R70s builds that prop only after the answer.
14. **The four interior records are declared in `or-io`.** They belong to the world family, whose typed roots the record allows by hand; the sheet of 2026-10-21 asked and was not answered ([rulings in force](Workbench_Plan.md#rulings-in-force)). Default: yes, in the data part of R70m.
15. **A story pack ships its maps in a nested archive**, asked on the same sheet and not answered. Default: yes, in R70q; nothing new is declared in `or-io`, since a build packs that archive today.
16. **A typed root is declared by hand outside the world, scenario and ped variation families**, which the record names; the occlusion metadata of an interior would be one. Default: no; R70x writes that file through a donor's type table. It waits: such a root is declared only after a yes.
17. **A changed game map in a shared repository.** Its source and the base beside it are the game's content, which the managed ignore block leaves out and Check before sharing lists. Default: the commit policy stands, and the toast of a first edit and the guide say so; the first part of R70u needs the setting on.
18. **No lane opens NodeRunner's sources**; the inventory of the parity document, read once, is all that is taken. Default: yes.

## Risks

- **The game refuses what a row writes.** Nothing here sees it: R70a first, and the sheet in the Build tab.
- **The world costs more than the frame has.** R70c measures before R70i builds on it.
- **An edit of a large map costs more than a drag has.** R70h measures the write and what History keeps before the tools are built.
- **A geometric judgement removes the wrong polygons.** R70l and R70o propose and the person confirms, every file is in Undo, and the game's file is the base to go back to.
- **A second resource ships the same game file**, and a server loads one of the two, whatever the tool. R70r finds it and merges a map, and R70k offers the script line that changes no file.
- **The install's cache states what a resource changed.** A cache file records each map's extents, each interior and each bound (R32c). What the game does when a resource moves an object past the old extents, adds an instance of an interior or changes a bound is not known here: each is a line of R70a's sheet.
- **A game map that does not come back from XML** cannot become a source. Its first edit is refused with a sentence, and the suite of R70h counts such maps.
- **A road node other regions name.** A link names its far node by region and index, so renumbering one region breaks its neighbours' links: R70p writes every region that names a renumbered node.
- **A file made from nothing carries what no shipped file does**: a zero signature for a class written with no donor, and a layout from the synthesiser of R62, whose clause that the game loads it is open. R70h makes a new map from a donor, and a row that grows an array adds its line to R70a's sheet.
- **An install of one edition and a build for the other.** The header says the mismatch and the first edit refuses a map of the other title ([what a world shows](#what-a-world-shows)).
- **The pull to copy.** The licence forbids it; a lane builds from these documents and from measurements of the install.
- **The shell grows a second application.** One viewport, one Details, one Undo, one Build, and the tests that hold the table of task screens to a row each.
- **Other titles.** The seam of R70c is where GTA VI and GTA V part; a rule that holds for one is not assumed of the other.

## Sources

- NodeRunner Studio, `https://github.com/HypnoZDev/NodeRunner`, at `1a04e46e`, read on 2026-11-25.
- [Workbench_Plan.md](Workbench_Plan.md), [Workbench_UX_Design.md](Workbench_UX_Design.md), [Workbench_Views_Design.md](Workbench_Views_Design.md#maps), [Project_Model_Design.md](Project_Model_Design.md) and [Source_Format_Design.md](Source_Format_Design.md) for what a project, a source and a task screen are.
- [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) for the streamed world that exists.
- [Roadmap.md](Roadmap.md): R32, R33, R41, R47, R62 and R69 for what is built and what each leaves open.
