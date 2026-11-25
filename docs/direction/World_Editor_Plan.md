# World editor plan

What OpenRage takes from NodeRunner Studio for GTA V, how each capability fits the World screen, the Map content and the project system, and the order it is built in.

Status: the plan of 2026-11-25, written on the owner's request of that day ([recorded](Project_Context.md#every-noderunner-studio-capability-for-gta-v-is-wanted-in-openrage-2026-11-25)). No row of it is built. [R70](Roadmap.md#r70-the-gta-v-world-editor) schedules it and says where each row stands; [the rulings asked](#rulings-asked-of-the-owner) are not given, and each row takes the default written beside its question until a sentence from the owner reverses it.

## What this plan is for

The owner's words: every capability of NodeRunner Studio, for GTA V, in OpenRage, with the world editor and the project system integrated and an experience that suits a person who makes maps. This document lists those capabilities from NodeRunner's own code, weighs each for and against, says where it lands in the tree, describes what the person sees, and orders the work into rows that each end on a check.

Three people use the result, and the first decides a trade-off:

- **The mapper**, for a FiveM server or for story mode, who adds, moves and removes objects, edits an interior and ships a resource.
- **The script developer**, who needs positions, model names and hashes in the form a script takes.
- **The curious**, who wants to see how the map is made: its levels of detail, its roads, its scenario points, its collision.

## What it rests on

- **NodeRunner Studio at `1a04e46e`**, its commit of 2026-11-24 and version 1.9.0, read on 2026-11-25: the README, the specification its author keeps, the interface strings, the test names and the header comments of its sources. What its README states and its code does not is said where it matters.
- **Its licence is PolyForm Strict 1.0.0**, which permits reading and use and forbids distribution and any work based on the software. OpenRage takes what a person can do with it and nothing of how it is written: no code, no text, no table and no asset ([Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources)). A row of this plan is built over `or-io` and the measurements of this tree, and a lane does not open NodeRunner's sources to build one.
- **The tree at `25af0f42`**, read on the same day: `crates/domain/or-world`, `crates/domain/or-maps`, the Map screen, the World screen that GTA VI has, the project container, and the rulings of [Workbench_Plan.md](Workbench_Plan.md#rulings-in-force).
- **Nothing here ran either program.** What NodeRunner draws was read, not watched, and no figure of its speed is repeated.

## Verdict

1. **The world is one screen with two doors.** The World room of the rail shows a GTA V install as it shows a GTA VI one, with nothing to set up and no project. The Map content of a project shows the same world with the project laid over it, and that is where an edit goes. One viewport, one set of panels, one camera code.
2. **The keystone is the GTA V world drawn with its models**, which the tree lacks: the Map screen draws boxes, and the streamed world is GTA VI's. Three rows build it ([R70b](#the-gta-v-world-index-r70b), [R70c](#the-gta-v-world-streamed-and-drawn-r70c), [R70g](#collision-as-a-query-r70g)), and every later row stands on them.
3. **A project keeps holding sources, and a changed game map is one of them.** The first edit of an object the game placed imports its map as a source that remembers the game's file. That memory is what NodeRunner's project lacks: it gives a three-way merge, a list of what changed, and a warning when a title update moves the file underneath.
4. **One vocabulary of edits.** Each thing a tool does is a named operation with an inverse: the step Undo takes back, a verb of `openrage-cli`, a line of History and, later, the message a second person receives.
5. **Rules run while the person works**, in the Problems panel, with a way to go to each finding. A check at export only is a check that arrives after the mistake is an hour old.
6. **The build is the project's build.** A FiveM resource and a story pack come out of the plan that every other content uses, and what a map adds is completeness: the manifest of its maps, the interior audio, the nested archive a story pack streams maps from.
7. **Nothing here proves the game loads a file**, and a world editor is worth what the game accepts. [R70a](#what-the-game-loads-r70a) is first: a sample resource for each kind of file a row writes, and a sheet the owner fills from a running game.
8. **Everything is in the app, not in a plugin.** A plugin draws lines one pixel wide and no mesh ([Plugin_ABI.md](../reference/Plugin_ABI.md)), so the interior builder, the walk mode and the thumbnail browser cannot be what a mapper needs there. This overturns the place [R47](Roadmap.md#r47-the-plugin-slate-after-the-first) gives them, and is [ruling 2](#rulings-asked-of-the-owner).
9. **Left out:** the earlier games, an updater, a link to a running game and window effects ([what is left out](#left-out-and-why)).

## Where the tree stands

What a row can stand on, and what is absent. The paths are the ones to read first.

| Capability | In the tree | Missing |
|---|---|---|
| Reading and writing `.ymap` and `.ytyp` | The typed world family in `or_io::meta`, the views of `crates/domain/or-world`, a canonical XML that compiles back | A map or a record made from nothing: an added entity is a copy of one the file holds |
| Moving, adding and deleting an entity | `or_maps::edit`, with the level-of-detail links renumbered and the extents grown in the same step | Entities only: no occluder, parked vehicle, light, room or portal |
| Maps that load together, and the manifest of a set | `MapGroup`, `crates/domain/or-world/src/manifest.rs`, `openrage-cli world manifest` | The project build writes no `_manifest.ymf` |
| Creating and deleting a map with every link kept | `WorkingSet` in `crates/domain/or-world/src/execute.rs` | Reached from `openrage-cli world` only |
| Which maps the game requests at an hour and a weather | `crates/domain/or-world/src/state.rs` | No screen uses it for GTA V |
| A 3D scene with a manipulator | The Map screen: Scene, Plan, Selection, `or_maps::gizmo`, snaps to a grid and to boxes | Boxes, no model; one set of a project; the ground is the highest box, not collision |
| A whole map streamed and drawn | The rules of `crates/domain/or-world/src/streaming.rs`, the instanced draws of `crates/runtime/or-renderer/src/world.rs`, the fly camera of `crates/runtime/or-renderer/src/fly.rs`, the budgets and the occluder buffer beside `src/state/gta6_worldview/` | Fed by a GTA VI install only |
| The game's picture | The Shaded viewport draws one model with GTA V's own passes, the install's timecycle, its sky and its water pass | At the scale of one model: one shadow tile, the sun alone |
| Collision | `BvhCollider` in `crates/runtime/or-simulation/src/collision.rs` casts a ray at one bound, for particles; `crates/io/or-io/src/gta5/bounds/build.rs` builds and rebuilds a bound | A query over the world, the bounds under a position, a snap to them, a screen that edits a polygon |
| Roads, navmesh, scenario points | `.ynd` and `.ynv` owned and saved, the navmesh drawn, scenario regions typed | Roads and scenario points are not drawn; no edit of the road graph |
| Which file answers a name | `crates/domain/or-depends`: the type file of an archetype, what a map needs, the maps that place a model, a map's bounds by the cache and by filename | The map or the bound at a position |
| The install as the game reads it | `crates/title/or-title/src/vfs/`: the DLC list, each pack's change sets, the mods overlay, a project's shadow folder | A level of DLC to stop at; a server's resources as a layer |
| A project, its sources, its build | Content and Builds, a FiveM resource, a story pack, an `.oiv`, rules, History, links, the plan and its claims ([Project_Model_Design.md](Project_Model_Design.md)) | The story pack is locked for maps; no scan of a server |
| A ped that moves | The Animation workspace and the dressed ped of the Wardrobe | A cutscene played, a point of life, a person who walks |
| Proof that the game loads a file | Nothing: no build says so ([Workbench_Plan.md](Workbench_Plan.md#held-back)) | A procedure and its results |

## What each product gets right

**NodeRunner, for the mapper.** The whole job is on one screen and starts with nothing to configure: the install is found and the map is there. An object lands on the game's real collision. A deletion follows its consequences, the distant model, the invisible wall and the hole in the occlusion, and shows them before it happens. The export is one action and looks first for another resource that ships the same file. What a script needs is one click away in the form the script takes. Checks are said in a mapper's words, and layers come in presets named after a task.

**OpenRage, already.** Both editions read and written from the file's own version. One model drawn with the game's shaders. Rules measured against the install, so a rule that fires on the game's own maps only warns. Sources as text that Git holds and compares. An edit written as it is made and a History kept beside the project, so there is no unsaved state to rescue. Every build read back file by file. The same work in `openrage-cli` and through the MCP server. Ten languages. Screens an agent can draw and check with no window.

**What is not copied.**

- A project of whole rewritten files with no memory of the game's: what changed is then guessed by matching objects, and a merge has no base.
- Every map read again at each start: the index of [R70b](#the-gta-v-world-index-r70b) is kept and keyed by what the install holds.
- Collision changes kept in a store apart from the entity edits, and a model editor with an undo of its own: here they are operations of one vocabulary and steps of one History.
- Checks that run at export only.
- Names known only from file names: the name index of the install answers a hash here.

## The capabilities, weighed

For each: what speaks for it in OpenRage, what speaks against, and the verdict. *Take* means as it is; *adapt* means the capability with a different shape; *later* means a late row; *leave* means not planned. The last column is the row of [the order](#the-order-it-is-built-in).

### Exploring

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| The whole map in 3D with its models, textures, interiors and levels of detail | The base of everything; the streaming rules and the world renderer exist | The largest row; GTA V's mount, index and materials are not the GTA VI ones | Take | R70b, R70c |
| Shadows, water, sky and an hour of the day | The install's timecycle is parsed already, which NodeRunner does not read; it is the positioning's third claim | A GTA V reader for `water.xml` is absent; the game's passes at world scale are unmeasured | Adapt: the install's own timecycle and weather | R70d |
| The map as of a DLC, and as story or multiplayer | Servers pin a build of the game; `state.rs` and the mount hold the change sets | A level to stop at is a new input of the mount | Take | R70b |
| Maps only a script requests, with the alternatives of one place hidden | Without it two versions of an apartment draw in one place | NodeRunner hides by naming rules; a rule by name is a guess | Adapt: a list the person toggles, grouped by the manifest's own groups, with a naming rule only as a default that is said | R70f |
| Search for anything: a name, a hash, a map, a type, a pasted position, a command | One field for every way of asking; the palette, the name index and Go to File exist | It must not become a second palette | Adapt: one palette, with places among its results | R70e |
| Layers for what the game hides | A mapper debugs with them; the data is read already | Each layer is a draw path to keep fast | Take, with presets | R70f |
| Life on scenario points | Shows what a point does without starting the game | A simulation of the game's own choice, which nothing here can check | Later | R70t |
| An archive explorer | OpenRage's is the fuller one | | Have: add Show in world to a row | R70e |
| GTA IV, its episodes and the first trilogy, read only | Their archives read in `or-io` | The owner asked for GTA V; [R57b](Roadmap.md#r57b-earlier-rage-titles) holds them unscheduled | Leave | |

### Editing objects

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Click to select, Ctrl for more, click again for what is behind | Expected | A pick by the pixel needs an id pass the world renderer lacks; a ray at boxes picks through walls | Take, by an id pass | R70i |
| A rectangle that selects objects | Mappers move groups; NodeRunner has it only for collision polygons | Ambiguous through walls: take what is visible | Add | R70i |
| A manipulator to move, turn and scale, with steps | Built: `or_maps::gizmo` | It draws through the plugin's line list | Have: draw it in the world, add a choice of the object's axes or the world's | R70i |
| Typed values for position, rotation and scale | Built, with keys for every handle | | Have | R70i |
| A group moved about its middle as one step | Expected | | Take | R70i |
| Drop to the ground, on the real collision, and follow it while moving | The first complaint NodeRunner's author lists; today's ground is a box | Needs the world's collision as a query | Take | R70g, R70i |
| Placing by the surface: stand on a floor, hang on a wall | Fast for props | A rule of thumb about what hangs | Take, with the rule shown and a key to switch it | R70j |
| Copy and paste, and a duplicate | Expected; NodeRunner has no general duplicate | | Take, both | R70i |
| Swap the model of a selection for another | Cheap and used often | | Take | R70i |
| Undo without limit and a History to click back through | Built for the project, kept across sessions | A step of the world must be a step of that History | Have | R70h |
| A rescue copy every few minutes | | An edit writes its source at once here | Leave: nothing is unsaved | |
| Every written file read back before it replaces the old one | Built: a build reads every file back, and a source edit refuses what does not compile to itself | | Have | |

### Removing what the game placed

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| A deletion that takes the distant models and corrects the child counts of their parents | Without it the far model stays; `delete_subtree` and the group's links exist | It rewrites maps the person did not touch | Take, with each file named before it happens | R70k |
| The collision of a deleted building removed from the bounds of its area | Without it a wall nobody sees stays | The riskiest write of the plan: which polygons belong to a building is a geometric judgement | Take, as a preview the person confirms, file by file | R70k |
| The occluders over a deleted building removed | Without it the scenery behind disappears | Same judgement | Take, same preview | R70k |
| A script line that hides a model, as the other way | No file changes, no conflict with another resource | Runs on every client, hides without removing collision | Take, as Copy for script | R70k |
| Objects that cannot be told from a twin are flagged and left | Honest | | Take | R70k |

### Collision

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Pick polygons, delete them, change their material, move them | Opens a passage, removes a wall; the bound's build and rebuild exist | A `.ybn` of the game is shipped whole under its name, a second source of conflicts | Take | R70l |
| The collision of a building follows it when it moves | Otherwise the wall stays behind | Same judgement as for a deletion | Take, as an option that says what it moved | R70l |
| The three versions of an area's collision kept alike | The game reads one for people, one for vehicles and one for the rest | Three writes for one gesture | Take | R70l |

### Interiors

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Rooms and portals drawn, named on hover | Read already | | Take | R70f |
| Entity sets shown and hidden, with the script line that turns one on | Read already; used daily on a server | | Take | R70m |
| Editing rooms, portals, entity sets and the objects of an interior | The owner's order puts props and interiors after maps | The four records are carried and not typed: declaring them is an `or-io` row with its gates | Take | R70m |
| A new interior, a copy under a new name, a second instance | The start of every interior | A record made from nothing | Take | R70m |
| The health of an interior | Already the shape of `or_maps::RULES` | | Take, as rules | R70m |
| The audio of an interior written at build: the occlusion file and the game data of its rooms | Without it sound passes through walls | Two files nothing here has written from nothing, and the game's verdict is needed | Take, last of its row | R70m |

### Doors

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Every door of a building found, paired when double | A server locks doors by script | A door is known by its model's name: a name the index lacks is a door missed | Take, saying how many models had no name | R70n |
| Copy for a door script, in the forms servers use | The developer's request | Forms that belong to other projects and change | Take, as templates kept as data | R70n |
| The arc a door opens through, and whether furniture blocks it | Catches a door that cannot open | A reading of the game's tuning | Take | R70n |
| A door replaced by another fitted to its frame | Quick | | Take | R70n |

### What a map holds besides objects

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Parked vehicles: place, turn, size, model, colours | In the map already, read | Not typed; flags nobody names | Take, with unnamed bits shown as bits | R70o |
| Timecycle zones: place, size, name, strength, hours | Same | Same | Take | R70o |
| Night lights seen from afar: place, colour, strength, hours | Same | A pair of maps that must stay in step | Take | R70o |
| Occlusion boxes: place and size; and boxes fitted to a building | Fixes the scenery showing through | The fit is a heuristic over collision | Take the edit; take the fit as a proposal the person accepts | R70o |
| A preview of what the occluders hide | The world viewer's occluder buffer exists | | Take | R70f |
| Roads: a node's speed and flags, its lanes, nodes and links added and removed | `.ynd` is owned and saved | An area file is rewritten whole; junctions are kept, not made | Take | R70p |
| Scenario points: move, type, hours, model set; add and delete | The region is typed | A region from nothing waits on [R62](Roadmap.md#r62-the-rbm-layout-synthesiser) | Take, in regions that exist | R70p |
| The navmesh seen, with warnings where a change leaves it wrong | Drawn already | No writer of new navmesh, here or there | Take the warnings; authoring is not planned | R70p |

### Models

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Remove, move or copy a part of a model; add a box or a plane | Small fixes with no round trip through Blender | OpenRage's model is a source of glTF and JSON: these are edits of that source | Adapt: operations on the model source | R70s |
| Replace, export and resize a texture | Built in the Texture workspace | | Have | |
| A prop made from an image | A sign or a badge in a minute | Author mode for drawables is [held back](Workbench_Plan.md#held-back) | Later, after the ruling | R70s |
| A door cut into a wall, and a building opened with a room behind it | Striking | Experimental there, and it touches the model, three collisions, the occluders and a new interior at once | Later, last | R70s |

### People and scenes

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Every ped with its outfits and every clip | The Animation workspace and the dressed ped do most of it | A browser of peds is new | Take | R70t |
| A cutscene played in its place with its camera | Read as metadata today | A player of its tracks is new, with no sound | Later | R70t |
| Walking the map: run, jump, climb, swim, stopped by walls | Finds a blocked path before a build | A character's motion written here, which is not the game's | Later, as a check and named as one | R70t |

### What comes in

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| A library of props shared by projects, with thumbnails and favourites | Links between projects and the thumbnail cache of `src/render/thumbnails.rs` exist | A library is a second kind of place beside the project | Adapt: a library is a project the others link | R70j |
| The game's own props by category | The fastest way to furnish | A shipped list of names is game content | Adapt: categories derived from the install by rule | R70j |
| A mapping imported from a folder, a zip or a package | Adopting a folder exists | A zip and an `.oiv` of another tool are new doors ([R50](Roadmap.md#r50-package-installation)) | Take the folder and the zip; the package with R50 | R70j |
| Sollumz and CodeWalker XML converted on import | How props and interiors leave Blender today | The ruling of 2026-09-22 that this XML is no target; a format with no specification | [Ruling 5](#rulings-asked-of-the-owner): import only | R70j |
| A watched folder that takes each new export | The loop a modeller wants | Needs the row above | With it | R70j |
| A report after each import of what does not find what | The import report panel and the rules exist | | Have: add the map rules | R70j |
| Files locked by a server's asset escrow recognised, kept whole and drawn as a box | Honest, and stops a build from failing on them | Never decrypted | Take | R70j |

### Projects and shipping

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| One project a resource, several open together | A project holds several contents and declares several builds, which is more | One project a window is a ruling | Have | |
| A FiveM resource in one action | Built: Ctrl+B | It lacks the manifest of maps and the audio lines | Have: complete it | R70q |
| Extents, links and counts recomputed at each write | Built for extents and links | Streaming extents of a game map never shrink | Have | R70q |
| Another resource of the server that ships the same file is named before the build | The conflict mappers lose days to | A server folder is a new place to read | Take | R70r |
| Its changes taken into this project by a three-way merge | Ends the conflict | The base is the game's file, which a project source keeps | Take | R70r |
| Any resource folder checked as the server will read it | Useful with no project | | Take, as a verb of `openrage-cli` first | R70r |
| A story pack | Built, and locked for maps | The game streams a pack's maps from a nested archive the build does not write | Take: write it | R70q |
| The pack installed into the mods folder with the DLC list edited | One action to test in story mode | It writes into the install; installing an `.oiv` is undecided since 2026-11-21 | [Ruling 7](#rulings-asked-of-the-owner): build only | |
| Positions copied as Lua, JSON or C# | The developer's daily need | | Take | R70e |

### Together

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Several people in one project, each seeing the others' edits, selection and camera | A team maps together | OpenRage reaches no network unless asked, and a relay is a service somebody runs | Later, in three steps | R70u |

### The shell

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Keys that can be changed, with a CodeWalker preset | Presets were left open on 2026-10-03; a mapper's hands know CodeWalker | | Take | R70v |
| Camera bookmarks | The GTA VI viewer has places | | Have: for GTA V too | R70e |
| Settings for quality, distance and camera | The viewer's preferences exist | | Have | R70c |
| Two interface languages | Ten here | | Have | |
| An updater that replaces the program | | OpenRage downloads and installs nothing on its own | Leave | |
| Translucent windows and detached panels | | Decoration; the dock and a second window exist | Leave | |

## How it fits the tree

### One world, two doors

The World room of the rail (`src/ui/panels/rail.rs`) appears while a GTA VI folder is set. It appears for a GTA V install too, and opens a tab that shows the install's map, read only: fly, search, layers, pick an object and read what it is. No project is needed, as [Open, Import and Add](Workbench_UX_Design.md#open-import-and-add) has it: an opened thing is a place to look.

The Map content of a project opens the same screen. What differs is the stack it draws and that its tools write. The Scene panel of the Map screen stops being a second scene: it is the world viewport, and a box is what an object draws as while its model loads or when no file answers its archetype. The Plan panel stays, as the map seen from above, and is how a person crosses the island in one drag.

### What a world shows

A world is a stack, read from the bottom:

1. The install, by the edition its files state, up to a level of DLC the person may choose, as story or as multiplayer.
2. Optionally a server: the resources of a folder the person names, read only, in the order the server starts them where a configuration says so.
3. The content the project links.
4. The project, last, as its build would ship it.

Each layer answers a name the way the game does, the newest winning, which is what the mount of `or-title` does for the install, the overlay and a project's shadow folder. The level of DLC and the server are the two new inputs. The stack is shown as a line of the screen's header, because what a mapper sees must be what the server will load, and a wrong stack is the first thing to rule out when it is not.

### How a project holds a change to the game's map

A project holds sources and no compiled game file. A map the project owns is a source. A map of the game that the person changes becomes one too, of the form that remembers where it came from: its XML, and beside it the game's file as it was.

- **The first edit imports.** Moving or deleting an object the game placed, with a project open, imports that object's map and applies the edit, as one step of History that Undo takes back whole, with the toast an import ends on. With no project the tool offers to start one. This is a new door into a project, so it is [ruling 3](#rulings-asked-of-the-owner).
- **What ships is what changed.** A source still equal to the game's file is left out of a build, which the build does already.
- **The kept file is the base.** What changed is a comparison of two files and not a guess. Another resource's copy of the same map merges three ways. When a title update changes the game's file, the check against the game names the source, and the same merge carries the project's changes onto the new file.
- **New objects go to a map of the project**, named in the header as the map that receives. A map whose extents grow past what one stream should hold is split by area at the build, with the layers of `crates/domain/or-world/src/partition.rs`, and a rule warns before that.

The cost is said plainly: a server that runs two resources shipping one game map loads one of them, whatever the tool. The plan's answer is to find it ([R70r](#other-resources-of-the-server-r70r)), merge it, and offer the script line that changes no file.

### One vocabulary of edits

Every tool of the world calls a named operation of `or-maps`: place, move, delete with its chain, set a field, add a room, cut a polygon. An operation says what it reads and writes, applies to the sources, and returns its inverse.

- Undo and Redo apply inverses, as steps of the project's History.
- `openrage-cli maps` gains a verb that applies the same operations from a file or from arguments, and the MCP server lists it with no change, since it lists every verb.
- A command of the screen offers Copy as command where its operation has a verb ([R69i](Roadmap.md#r69i-the-automate-room)).
- The live session of [R70u](#working-together-r70u) sends them.

The operations grow from `or_maps::edit` and from the working set of `or-world`, which today serve the Map screen and `openrage-cli world` apart.

⚠ An operation writes its source when it is made, and a source of a large game map is a large XML: the cost of one drag on the biggest map of the install is measured in [R70h](#one-vocabulary-of-world-edits-r70h) before the tools are built on it, and a worker writes behind the screen if the measure says so.

### Collision as a query

Dropping to the ground, placing on a surface, the clearance of a door, walking, and the judgement of which polygons belong to a building all ask the same questions of the same data: what does this ray hit, what does this swept shape touch, which bounds lie under this point. They are answered once, in the domain layer, over the bounds `or-io` declares, with the bounds of a position found by the index. The collider that particles use becomes a caller of it, so the tree has one.

### Where the code goes

| What | Where | Why there |
|---|---|---|
| Records a row edits that are carried today: rooms, portals, entity sets, the time cycle modifiers of an interior, parked vehicles, occluders, the far lights | `or-io`, declared with the engine's names | D15 and D16; each is a format row with its gates |
| A GTA V reader for `water.xml`, a writer for the timecycle | `or-io` | Bytes only |
| The level of DLC, story or multiplayer, a server as a layer | `or-title` | It knows where a file lives |
| The world index: the maps at a position, the bounds at a position | `or-depends`, beside the indexes it holds | It answers which file |
| Collision queries, the edit of a room or an occluder, the fit of occluders, door geometry | `or-world` | Rules of the world over typed data |
| The operations, the rules, the scene, placement | `or-maps` | The content kind |
| What a server's resources ship and claim | `or-project`, beside adoption and the claims | It is the plan's vocabulary |
| A walking character | `or-simulation` | Runtime, over the collision query and `or-anim` |
| The session of several people | A new foundation crate, and nothing below the app reaches a socket | No layer above needs to know the wire |
| The streamer for GTA V, the layers, the tools, the panels | The app: a world view shared with GTA VI behind one seam that says where its maps and models come from | The shell's budget for each file of `src/` holds |

A new edge that points up is refused as ever: `or-maps` does not see the renderer, and the manipulator stays maths.

### The editor, the CLI and plugins

What the editor can do, the CLI can do. Each row names its verbs, and the writing ones are one-shot commands of `openrage-cli`, not routes of the server, which stays a reader ([CLI_Server_Protocol.md](../reference/CLI_Server_Protocol.md)). Plugins keep what they have: a validator and an emitter extend a map's build, and an overlay of lines draws in the world as it draws on a model.

### The picture and the frame

The first cut of [R70c](#the-gta-v-world-streamed-and-drawn-r70c) draws with what the world renderer has, and [R70d](#the-worlds-day-sky-water-and-shadows-r70d) brings the install's own day. Whether the whole map is drawn through the game's own passes, as one model is, is a measurement of that row and [ruling 9](#rulings-asked-of-the-owner).

No frame time is promised here. R70c sets one from a measure on the owner's machine, the Statistics panel shows the frame by phase and the caches against their budgets ([R59f](Roadmap.md#r59f-performance-and-cache-tuning)), and the index of R70b is held to a time for a second start.

### What cannot be checked here

No agent runs the game. A row is checked by what the tree can see: files read back, rules, the game's own files put through the same writer, screens drawn with no window. Whether the game shows the object, opens the door or plays the echo is a line of [the acceptance sheet](#what-the-game-loads-r70a), and until that line is filled the screen says "not checked in game" where the person builds.

## What the mapper meets

The screen the rows build, said as a person meets it. None of it is in the tree, and the words of a control are settled by the row that builds it.

### The first five minutes

1. OpenRage opens on the install it remembers. The start page has a tile *Explore the map*, and the rail has World.
2. The world appears where the person last was, or over the city on a first visit, drawn at once from far models while near ones arrive. A first visit builds the index behind a line of the status bar; flying works meanwhile.
3. The palette takes a pasted position, a model name or a hash and goes there.
4. A click selects. Details says what the object is, which map places it, which file declares it, and offers Copy for script.
5. Pressing a move key on a game object with no project open shows one line: *This object belongs to the game. Start a map project to change it*, with the button. The dialog asks a name and where the map is for, a FiveM server or story mode, and nothing else.
6. The object moves. Ctrl+B builds. The Build tab says what was written, what was left out as unchanged, and what was not checked in game.

### The window

```text
+- Home | Game | World | Project | Automate ------------------------------------ [Build: harbour v] -+
| [World] [harbour: Map x] [prop_bench.ydr]                                                          |
| Legacy . up to <pack> . multiplayer . server: <folder> . project: harbour . new: harbour_props     |
+--------------+--------------------------------------------------------------+----------------------+
| Outline      | [1 Select][2 Move][3 Turn][4 Scale][5 Collision]             | Details              |
|  Near me     | [Ground] [Step 0.5 m] [World axes]                           |  <model name>        |
|  v harbour   |                                                              |  in harbour_props    |
|     bench    |                                                              |  position   x  y  z  |
|     crate    |         the world, with the manipulator                      |  rotation   x  y  z  |
|  v game maps |         on what is selected                                  |  scale      xy  z    |
|     <map>    |                                                              |  level of detail     |
| Layers       |                                                              |  [Copy for script v] |
|  (Mapping v) |                                                              |  Uses . Placed by    |
|  [x] Objects | 12:00 . Clear sky                    x <n>  y <n>  z <n>     |                      |
+--------------+--------------------------------------------------------------+----------------------+
| Problems 3 | Build | History | Library | Jobs                                                      |
+----------------------------------------------------------------------------------------------------+
```

The line under the tabs is the stack, each part a button that changes it, and its end names the map that receives a new object. The bottom strip gains Library beside the tabs every task screen has.

### Tools, keys and the pointer

- **Five tools on the keys 1 to 5**: select, move, turn, scale, collision. Ctrl+1 to Ctrl+9 stay the tabs. The same tools are buttons with their key in the hint.
- **The camera flies while the right button is held**, with the keys the GTA VI viewer uses, and orbits the selection with Alt. Outside that, a letter is a command and never a movement, so a key typed after a click does one thing.
- **Ctrl reverses the snap**, as the manipulator does today. The step sizes are in the bar and remembered.
- **Every handle has a typed field and a key**, the rule the Map screen keeps.
- **End drops to the ground.** A toggle makes an object follow the ground while it moves.
- **Escape** ends a drag as it started, then ends a placement, then clears the selection, in that order.
- **Presets** of keys: OpenRage's, and one for hands that know CodeWalker ([R70v](#the-mappers-keys-settings-and-guide-r70v)).

### Selecting

A click picks what is under the pixel; a second click at the same place picks what is behind it. A decal, a painted shadow and a pane of glass give way to the surface under them. Ctrl adds, a drag on empty space draws a rectangle that takes what is visible in it, and the Outline selects by name. While a layer is shown its things are picked before objects, so a road node on a road is the node. A selection of several kinds says so in Details and offers only what all of them take.

### Placing

The Library tab lists the props of the project, of what it links, and of the game by category, with thumbnails, a search and favourites. A prop is dragged into the world or chosen and then clicked into place: a ghost follows the pointer on the surface under it, R turns it, each click places another and Escape ends. What was placed is selected, so the next key adjusts it.

### Deleting

Delete on an object of the project removes it. Delete on an object the game placed opens a preview and not a question: what goes (the object, its far models, parents left with nothing), what stays and why (a far model that also stands for a neighbour), the collision and the occluders proposed for removal, drawn in the world in two colours with a box to untick for each file, and the files the build will then ship. One confirmation, one step of Undo.

### Finding

One palette. It lists commands, files, and now places: a position in any form a script writes it, a hash in hexadecimal or as the signed number a script prints, a model, a map, a type. Enter goes there in the world; on a model nothing places, it opens the model. Bookmarks are places with a key.

### Layers

A panel of switches with presets named after tasks: map only, mapping, roads and traffic, interiors, occlusion, parked vehicles, night lights, atmosphere, sound. A layer draws within a radius of the camera that Settings holds. The levels of detail are layers too, so a mapper can look at the far city alone. Maps that only a script requests are a list with a search, and where two of them are versions of one place the list says so.

### Details

One panel ([R69g](Roadmap.md#r69g-one-details-panel)): the object in a mapper's words first, with the raw fields behind Show internals. Rotation is shown in degrees and stored as the file stores it. A flag with a name is a checkbox and a bit without one is shown as a bit. Uses and Placed by are a click away. Copy for script is a button with the last form used and a list of the others.

### Problems

The rules run on the project as it is edited. A finding says what is wrong in a sentence, goes to the place when clicked, and offers a fix where the fix is safe, as Fix all safe does for the other contents. The Build control carries the count. Nothing is checked only at the build except what needs the built files.

### Building and the server

Build is the window's one Build control. The first build to a server asks for the server's resources folder and remembers it for this machine, never in the project file. Before it writes, the build names any other resource that ships one of its files and offers to compare, merge or go on. After it writes, it shows the line to add to the server's configuration and what it did not check in game.

### Empty states and refusals

Every panel with nothing to show offers the action that fills it. A tool that cannot act says why in a sentence at the pointer: an interior cannot be scaled, an Enhanced file cannot go to a FiveM resource as it is and the build converts it, a locked file is kept whole.

### Words, languages and reach

Screens say Legacy and Enhanced, and call a `.ymap` a map and a `.ytyp` a type file, as the Map screen does. Every sentence goes through the catalogs of every interface language, French checked against the validated vocabulary ([Interface_Languages.md](../guides/Interface_Languages.md)). Every control has a name a screen reader speaks, colour is never the only sign of a layer or a level of detail, and the axes of the manipulator carry their letters.

## The order it is built in

A row is one lane and usually one pull request, and ends on a check that failed once before it passed. Sizes are S, M, L and XL as [Workbench_Plan.md](Workbench_Plan.md#the-second-plan-from-the-audit-of-2026-10-08) uses them. A stage starts when the rows it depends on have merged; rows of one stage that share no file run together.

| Row | Content | Depends on | Size | Stage |
|---|---|---|---|---|
| R70a | What the game loads: the sample resources and the sheet | Ruling 1 | S | 1 |
| R70b | The GTA V world index | | L | 1 |
| R70g | Collision as a query | R70b | L | 1 |
| R70h | One vocabulary of world edits, the first edit that imports, a map from nothing | | L | 1 |
| R70c | The GTA V world streamed and drawn | R70b | XL | 2 |
| R70e | Find anything and go there | R70b, R70c | M | 2 |
| R70i | Selecting and moving in the world | R70c, R70g, R70h | L | 2 |
| R70q | A build a server and a story install take | R70h | L | 2 |
| R70d | The world's day, sky, water and shadows | R70c | L | 3 |
| R70f | Layers of what the game hides | R70c | L | 3 |
| R70j | A library of props and what comes in | R70i | L | 3 |
| R70k | Removing what the game placed | R70g, R70i | L | 3 |
| R70r | Other resources of the server | R70q | M | 3 |
| R70l | Editing collision | R70f, R70i | L | 4 |
| R70m | Interiors | R70f, R70i | XL | 4 |
| R70n | Doors | R70g, R70i | M | 4 |
| R70o | Parked vehicles, zones, lights and occluders | R70f, R70i | L | 4 |
| R70p | Roads, scenario points and the navmesh | R70f, R70i | L | 4 |
| R70s | The model workshop | R70i | XL | 5 |
| R70t | Peds, cutscenes, life and walking the map | R70c, R70g | XL | 5 |
| R70u | Working together | R70h, ruling 6 | XL | 5 |
| R70v | The mapper's keys, settings and guide | R70i | M | 5 |

Four points on the way are worth naming, because each is something a mapper can be told:

- **After stage 1** nothing is on screen, and three things are true that were not: the tree can say what is at a position, what a ray hits, and what an edit is.
- **After stage 2** a person opens the map, finds a place, drops a prop on the ground and builds a resource. That is the first thing to hand to a mapper.
- **After stage 3** the daily job is covered: a library, a clean deletion, the layers, the server's conflicts.
- **After stage 4** every kind of thing a map holds can be edited.

## Briefs

Each brief says what to read first, what to build and the checks the row ends on. [What every wave does](Workbench_Plan.md#what-every-wave-does) applies to each: documents travel with the code, every sentence is translated, screens are checked on `just shots`, and a control joins [the control inventory](Workbench_UX_Design.md#control-inventory).

### What the game loads (R70a)

Read [Workbench_Plan.md](Workbench_Plan.md#held-back) and the open clauses of [R62](Roadmap.md#r62-the-rbm-layout-synthesiser), [R51c](Roadmap.md#r51c-dlc-pack-creation) and [R33e](Roadmap.md#r33e-navmesh-index-stability).

Build a sample project whose build is one small resource for each kind of file a row of this plan writes, each placed where a person can walk to it, and a guide page with the steps to load it on a FiveM server and in story mode and a sheet of what to look for: the object is there, the far model is gone, the wall is gone, the door opens, the sound stops at the wall. The sheet records the date, the game's build and the result for each line. The Build tab reads the sheet's lines for the kinds of file a build wrote and says which are not checked.

Checks: the sample builds headless and every file reads back; a kind of file a row writes and the sample lacks fails a test; the sheet's format is read by a test. The results are the owner's.

### The GTA V world index (R70b)

Read `crates/title/or-title/src/vfs/`, `crates/domain/or-depends`, `crates/domain/or-world/src/state.rs` and how the GTA VI viewer keeps its scan.

Build the index of a mounted GTA V install: every map with its extents, parent and the group that requests it; every archetype with its type file and box; the bounds of each map from the cache files and by filename; and two queries by position, the maps and the bounds. Add the level of DLC to the mount and the choice of story or multiplayer. Keep the index on disk beside the caches, keyed by what the install holds, and rebuild only what changed. Add the verbs `openrage-cli world at`, which answers a position, and `openrage-cli world index`, which builds the index and reports it.

Checks: over a mounted install the index names every map and type file the mount lists, and a position inside a known map's extents answers that map (an install suite, in the slow tier, named in the skip ledger without one); a second start reads the kept index and opens no archive that did not change; a level of DLC hides what a later pack adds, shown on a synthetic mount.

### The GTA V world streamed and drawn (R70c)

Read [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) and `crates/domain/or-world/src/streaming.rs`.

Draw a GTA V install through the world renderer. Separate what the World screen takes from a title, where maps, models and textures come from and how a material is drawn, from what it shares: the selection of levels, the budgets, the wait for textures, the occluder buffer, the camera. Feed the shared part from R70b. Draw interiors and their objects. Make the Map content's Scene this viewport. Settings for distance and quality are the viewer's.

Checks: a synthetic world of a few maps with parent and child levels draws the level the rules select at each distance, through real frames with no window; the GTA VI screens draw as before the seam; a frame over the city on the owner's machine is measured and its budget recorded where the Statistics panel reads it. The picture itself is the owner's to judge.

### The world's day, sky, water and shadows (R70d)

Read [Viewport_Shading_Modes.md](../features/Viewport_Shading_Modes.md) and the timecycle drivers under `src/state/drivers/`.

Light the world from the install's timecycle and weather at an hour the person sets. Declare a reader for GTA V's `water.xml` in `or-io` and draw the water it describes. Give the world its shadow cascades and its sky. Measure what drawing the whole map through the game's own passes costs, and report it for ruling 9.

Checks: the reader opens the water file of both installs and saves its fixed point; the same hour and weather give the same sun on a model in the Shaded viewport and in the world.

### Find anything and go there (R70e)

Read the command registry and Go to File ([R59j](Roadmap.md#r59j-one-command-registry), [R24b](Roadmap.md#r24b-go-to-file)).

Add places to the palette: a position in the forms scripts write, a hash in each spelling, a model, a map, a type. Add Show in world to a file's row and to a row of Uses and Placed by. Add bookmarks with keys. Add Copy for script in Lua, JSON and C#, for one object or a list, kept as templates in data. Add `openrage-cli world find`, which answers the same questions.

Checks: each form of a position parses to the same point, each written first as a case that failed; a hash typed three ways finds one model; a copy in each form parses back to the position it came from.

### Layers of what the game hides (R70f)

Read `src/render/navmesh.rs` and the read accessors of `or-world`.

Draw, each within its radius: road nodes and links by category, navmesh by category, scenario points with their heading, collision by material and by version, rooms and portals, occluders with a preview of what they hide, parked vehicles, timecycle zones, far lights, and sound zones read from the game data. Add the presets, the list of maps only a script requests, and the pick order.

Checks: each layer draws a synthetic file's records at their places through real frames; a preset shows the layers it names and no other; a layer off costs no draw.

### Collision as a query (R70g)

Read `crates/runtime/or-simulation/src/collision.rs` and `crates/io/or-io/src/gta5/bounds/build.rs`.

Build, over the declared bounds, a ray, a swept sphere and a box overlap, with the hit's material; a set of bounds streamed by position from R70b; and the ground under a point that ignores walls. Make the particle collider call it. Add `openrage-cli world ground`, which answers a position.

Checks: each primitive of a bound is hit where arithmetic says, on bounds built by hand; over an install, the ground under a position on a known road is within a step of the road's nodes there (slow tier); the particle preview's collisions are unchanged.

### One vocabulary of world edits (R70h)

Read `crates/domain/or-maps/src/edit.rs`, `crates/domain/or-world/src/execute.rs` and [what project history builds](Workbench_UX_Design.md#what-project-history-builds).

Name the operations and give each its inverse and a form as data. Build the first edit that imports a game map as a source with its base. Build a map and an entity made from nothing, which needs the world family built from empty in `or-io`. Bring the working set's create, delete and delete with the chain behind the same operations. Add `openrage-cli maps apply`, which takes a list of them. Measure the cost of one edit on the largest map of the install.

Checks: every operation followed by its inverse leaves the sources byte for byte as they were, held by a test that fails when an operation has no inverse; a list applied by the CLI and by the screen gives the same sources; a new map reads back as its root and passes the rules.

### Selecting and moving in the world (R70i)

Read `crates/domain/or-maps/src/gizmo.rs` and [what the Map screen builds](Workbench_UX_Design.md#what-the-map-screen-builds).

Build the pick by pixel with its second click, the rectangle, the group move, the choice of axes, copy, paste and duplicate, the swap of a model, the drop to the ground and the follow, over R70g. Draw the manipulator in the world. Build the line that offers a project and the toast of the first edit.

Checks: `the_maps_screen_on_screen` extended to a drag in the world on a model, one step of Undo each; a pick through a hole in a fence takes what is behind it and a pick at a wall does not; an object dropped lands on the collision under it and not on a roof above.

### A library of props and what comes in (R70j)

Read the links and adoption of [Project_Model_Design.md](Project_Model_Design.md#what-links-adoption-and-templates-build) and `src/render/thumbnails.rs`.

Build the Library tab over linked projects and the game's props by derived category, the ghost and placement by surface, a zip as a place to adopt from, the recognition of locked files, and the map rules in the import report. With ruling 5, the reader of the XML a Blender export writes and the watched folder.

Checks: a prop dragged from a linked project builds with only the files it needs; a locked file survives a build byte for byte and is named; a zip that reaches outside its folder is refused.

### Removing what the game placed (R70k)

Read `MapGroup` and `delete_subtree`.

Build the deletion's preview and its operation: the chain, the parents' counts, the files touched. Build the proposal of polygons and occluders to remove, as data the person edits before it applies. Add the copy of the script line that hides a model, and the flag for twins.

Checks: over an install, deleting any object with its chain leaves every link and count valid, as `executing_a_plan_keeps_every_link` holds for a map; on bounds built by hand, the proposal takes the walls of the building and leaves the ground; undo restores every file.

### Editing collision (R70l)

Read `crates/io/or-io/src/gta5/bounds/` and how the bound is rebuilt.

Build tool 5: pick polygons, a rectangle that takes the visible ones, delete, set a material from the game's list, move with shared corners following, and the option that moves a building's collision with it. Write the three versions together.

Checks: an edited bound passes the validation that refuses what the game would misread, and reads back with the polygons expected; the three versions stay alike in what was edited.

### Interiors (R70m)

Read [what wave P44 built for Maps](Workbench_Views_Design.md#what-wave-p44-built-for-maps), which lists the members the install states for each record.

Declare rooms, portals, entity sets and the interior's time cycle modifiers in `or-io` with the engine's names, with the gates. Build their edit, a new interior, a copy under a new name, a second instance, the assignment of an object to a room, and the health rules. Build the entity set switches and their script lines. Last, write the interior's audio files at build.

Checks: the `or-io` gates for the type file with the four records; every interior of an install reads typed and saves to its fixed point; each health rule fires on an interior built to break it; the audio files read back through the readers that open the game's.

### Doors (R70n)

Build the search of doors by model name with the count of unnamed models, the pairing, the copy forms as templates, the arc and the clearance over R70g, the fitted replacement and the door rules.

Checks: two leaves that face each other pair and two that do not stay single; a door behind furniture is reported blocked and the same door in an empty room is not.

### Parked vehicles, zones, lights and occluders (R70o)

Declare the four records in `or-io`. Build their place, move, size and fields with the manipulator's limits for each, the far lights' two maps kept in step, and the occluder fit as a proposal.

Checks: the `or-io` gates; every record of an install reads typed and saves to its fixed point; a fitted box lies inside the collision it was fitted to, on a building built by hand.

### Roads, scenario points and the navmesh (R70p)

Build the edit of a road node's fields and lanes, nodes and links added and removed with the renumbering, and of a scenario point with its chain following. Build the rules: a node alone, a link one way, a point in the air, a building on walkable ground.

Checks: an edited area file reads back with every link resolving; a deleted node leaves no link to it; each rule fires on a file built to break it.

### A build a server and a story install take (R70q)

Read [Dlc_Pack_Creation.md](../features/Dlc_Pack_Creation.md) and `crates/domain/or-world/src/manifest.rs`.

Add to a map's contribution the manifest of its maps, the line that declares the resource a map, and the audio lines. Write the nested archive a story pack registers for its maps, and unlock the cell. Split a map past its extents at the build.

Checks: the derived manifest equals the shipped one for the maps of an install where the derivation agrees today; a built pack lists its maps where the install's own packs list theirs; the cell says yes only with the archive written.

### Other resources of the server (R70r)

Read `crates/domain/or-project/src/adopt.rs` and the plan's claims.

Build the scan of a resources folder into what each resource ships and claims, the finding that names two resources shipping one file, the three-way merge of a map with the game's file as base, and `openrage-cli project check-server` and `check-resource`, the second for any folder.

Checks: two resources that change different objects of one map merge with no conflict and two that move the same object report it; a resource missing a listed file fails the check and a sound one passes.

### The model workshop (R70s)

Read [Source_Format_Design.md](Source_Format_Design.md) and [Mesh_Interchange.md](../features/Mesh_Interchange.md).

Build, as operations on a model source: remove a connected part in every level, move and copy a part, add a box or a plane with a material of the model. After the ruling on author mode, a prop from an image. Last and marked experimental, the opening cut into a wall.

Checks: a model with a part removed recooks, reads back and draws; the same part is gone from every level.

### Peds, cutscenes, life and walking the map (R70t)

Build a browser of peds with outfits and clips over the Animation workspace; then a player of a cutscene's tracks in the world; then what a scenario point would spawn at the hour shown; then a character that walks, climbs and swims over R70g, named a check of the path and not the game.

Checks: the character stops at a wall and steps a kerb on bounds built by hand; a cutscene's cuts fall at the times its file states.

### Working together (R70u)

Three steps, each usable alone. First, a merge driver for a map source, so two people who share a repository merge by object and not by line. Second, a session on a direct connection: the host orders the operations of R70h and sends them, with each person's selection and camera. Third, with ruling 6, a relay that forwards bytes it cannot read.

Checks: two edits that cross are ordered the same for every person; a person who joins late has the same sources byte for byte.

### The mapper's keys, settings and guide (R70v)

Build presets of keys with one for CodeWalker's, the settings a mapper changes on one page, a tour of the world, a chapter of the guide and a Map template that opens on the world.

Checks: no preset binds one key to two commands of one place; every step of the tour points at a control that is drawn.

## Rulings asked of the owner

Each has a default the rows take. A sentence reverses it.

1. **Who loads a build in the game.** Default: the owner, from the sheet of R70a, once a stage; no build says more than the sheet does.
2. **The interior builder, the walk mode and the thumbnail browser are built in the app**, where R47 files them as plugins. Default: in the app, since a plugin cannot draw a mesh.
3. **The first edit of a game object imports its map into the project** without a question, as one step with its toast. Default: yes.
4. **A server's resources folder is a place OpenRage reads**, and a build's folder may be set inside it, where a marker protects every folder the build did not write. Default: yes, remembered for the machine and never in the project file.
5. **The XML a Blender export writes is read on import**, for drawables, textures, bounds, type files and maps, landing as sources, and is never written or stored. It reopens the ruling of 2026-09-22 for import only. Default: yes; without it a prop reaches OpenRage as glTF, with no collision and no type file.
6. **A live session reaches the network when a person starts one**, and a relay is a program a team runs for itself. Default: yes to both; OpenRage runs no service.
7. **A story pack is installed into the mods folder from the build.** Default: no; the build writes the pack, and installing waits for the ruling the entry of 2026-11-21 left open.
8. **The tools are on the keys 1 to 5 and the camera flies on the right button.** Default: yes, with a CodeWalker preset.
9. **The whole map is drawn through the game's own passes**, as one model is. Default: not in the first cut; R70d measures it.
10. **Forms for other projects' door scripts are kept as templates in data.** Default: yes, each named after what it is for.
11. **The game's props are grouped by rules over the install**, and no list of names is shipped. Default: yes.
12. **The earlier games and an updater are left out.** Default: yes.

## Left out, and why

- **GTA IV, its episodes and the first trilogy.** The request is GTA V, and R57b keeps them a future goal.
- **An updater.** It downloads and installs on its own.
- **A link to a running game.** A non-goal of [the roadmap](Roadmap.md#non-goals), which the walk mode does not touch: it runs no game.
- **A rescue copy.** Nothing is unsaved.
- **Authoring navmesh**, grass, an entity's extensions on screen and a cinematic camera: NodeRunner has none either, and the first three are data the Data workspace edits.
- **Translucent windows.**

## Risks

- **The game refuses what a row writes.** Nothing here sees it. R70a first, and the sheet in the Build tab.
- **The world costs more than the frame has.** The GTA VI viewer's budgets are the start; R70c measures before R70i builds on it.
- **A geometric judgement removes the wrong polygons.** R70k and R70l propose and the person confirms, every file is in Undo, and the game's own file is the base to go back to.
- **A second resource ships the same game file.** No tool ends it; R70r finds it and merges.
- **The pull to copy.** The licence forbids it. A lane builds from this document and from measurements of the install.
- **The shell grows a second application.** One viewport, one Details, one Undo, one Build, and the tests that hold the table of task screens to a row each.
- **Other titles.** The seam of R70c is where GTA VI and GTA V part; a rule that holds for one is not assumed of the other.

## Sources

- NodeRunner Studio, `https://github.com/HypnoZDev/NodeRunner`, at `1a04e46e`, read on 2026-11-25.
- [Workbench_Plan.md](Workbench_Plan.md), [Workbench_UX_Design.md](Workbench_UX_Design.md), [Workbench_Views_Design.md](Workbench_Views_Design.md#maps) and [Project_Model_Design.md](Project_Model_Design.md) for what a project and a task screen are.
- [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md) for the streamed world that exists.
- [Roadmap.md](Roadmap.md): R32, R33, R41, R42, R47, R48, R62, R67 and R69 for what is built and what each leaves open.
