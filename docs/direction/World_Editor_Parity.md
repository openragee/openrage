# World editor parity

What NodeRunner Studio does for a GTA V map, capability by capability, what OpenRage has of it today, and what the rows of R70 take, adapt, add, defer, hold for a ruling or leave out, each with its reason.

Status: snapshot of 2026-11-25; [the roadmap](Roadmap.md#r70-the-gta-v-world-editor) is current.

## How to read it

The owner asked on 2026-11-25 for every capability of NodeRunner Studio, for GTA V, in OpenRage ([recorded](Project_Context.md#every-noderunner-studio-capability-for-gta-v-is-wanted-in-openrage-2026-11-25)). This document accounts for that request, one row for each thing a person can do with NodeRunner. [World_Editor_Plan.md](World_Editor_Plan.md) says how the rows fit the tree, orders them and briefs each, [World_Editor_UX_Design.md](World_Editor_UX_Design.md) says what a mapper would meet on screen, and [R70](Roadmap.md#r70-the-gta-v-world-editor) holds each row's check and where it stands.

**The inventory was read once**, from NodeRunner Studio at revision `1a04e46e`, on 2026-11-25: its README, the specification its author keeps, its interface text, and the names and header comments of its sources and tests. Its licence, PolyForm Strict 1.0.0, permits reading and non-commercial use and forbids distribution and any work based on the software. OpenRage therefore takes what a person can do with it and nothing of how it is written: no code, no interface text, no table and no asset ([sources](../guides/Licence_And_Provenance_Policy.md#sources)). Every capability below is said in OpenRage's own words, a row is built over `or-io` and measurements of the install, and a lane that builds one does not open NodeRunner's sources, a rule the plan puts to the owner as a ruling. Neither program was run for this document: what NodeRunner draws was read and not watched, and no figure of its speed is repeated.

Each table has the same columns:

- **Capability** is what a person does with NodeRunner, or, where the verdict is Add, what another row of this document needs.
- **For and Against** weigh it for OpenRage. A cell describes the tree, as it was at `25af0f42` on 2026-11-25, only where it says today or names a file. Everything else in a row is proposed, and none of it is built.
- **Verdict** is one of the seven words below, with the shape the capability takes where that needs saying.
- **Row** names the rows of R70 that build it, by the letters of the roadmap. It is empty where nothing is built.

The verdicts:

- **Have**: the tree does it today. Where a row is named, that row carries it into the world screen and builds nothing else for it.
- **Take**: built as a person meets it in NodeRunner, by OpenRage's own means.
- **Adapt**: the same capability in another shape, which the cell states.
- **Add**: the inventory does not show it in NodeRunner, and a row builds it because a capability taken here needs it.
- **Later**: taken, as a part that lands last in its row, on its own check, after the parts other rows depend on.
- **Waits**: recommended, and not built before the owner answers, because its default reopens a dated owner decision or builds what [Workbench_Plan.md](Workbench_Plan.md#held-back) holds back. The cell names the ruling, and [the plan's list of rulings](World_Editor_Plan.md#rulings-asked-of-the-owner) says of it "It waits".
- **Leave**: not planned. [Left out, and why](#left-out-and-why) gives the reason.

Five subjects wait: the interior builder, the walk mode and the thumbnail browser built in the app and not as plugins (ruling 2), which [R47](Roadmap.md#r47-the-plugin-slate-after-the-first) files as plugins; reading the XML a Blender export writes (ruling 5), which reopens the ruling of 2026-09-22; a drawable made from nothing, which is author mode (ruling 13); a typed root declared by hand outside the families the record names (ruling 16); and installing into the game's mods folder (ruling 7), left undecided on 2026-11-21. Every other row takes the default the plan writes beside its question, and a cell names the ruling where a default answers a question the owner was asked and has not answered.

Nothing here says the game loads a file. No agent runs GTA V, a build says only that every file it wrote reads back, and R70a is the sheet the owner fills from a running game.

## What each product gets right

**NodeRunner, for the mapper.** The whole job is on one screen and starts with nothing to configure: the install is found and the map is there. An object lands on the game's real collision. A deletion follows its consequences, the far model, the wall nobody sees and the hole in the occlusion, and shows them before it happens. The export is one action and looks first for another resource that ships the same file. What a script needs is one click away in the form the script takes. Checks are said in a mapper's words, and layers come in presets named after a task.

**OpenRage, today.** Both editions are read and written from the file's own version. One model is drawn with the game's shaders, and for GTA VI a whole world is streamed and lit from the install's own day. Rules are measured against the install, so a rule that fires on the game's own maps only warns. Sources are text that Git holds and compares. A map edit writes its source as it is made, so nothing is unsaved, and the project's History is kept beside the project. Every build is read back file by file. The same work runs in `openrage-cli` and through the MCP server. The screens are in every catalog of `src/i18n/languages`, and an agent can draw and check them with no window.

**What is not copied.**

- A project of whole rewritten files that keeps no copy of the game's. What changed is then found by pairing objects with the install's file of the day, so the base of a comparison and of a merge is whatever the install holds. Here a changed game map keeps the game's file beside it as its base.
- Every map read again at each start. The index of R70b is kept on disk and keyed by what the install holds.
- Collision changes kept in a store apart from the entity edits, and a model editor with an undo of its own. R70h makes them operations of one vocabulary and steps of one History.
- Checks that run at export only. The checks below are rules of the Problems panel and run while the person works.
- Names known only from file names. The name index of the install answers a hash here.

## Exploring

Seeing the map, moving through it and finding a thing in it. The world drawn is R70b and R70c, its day R70d, the search R70e and the layers R70f.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| The whole map in 3D with its models, textures, interiors and levels of detail | Every other row stands on it; the streaming rules and the world renderer exist today and draw a GTA VI install | The largest row; GTA V's mount, index and materials are not GTA VI's, and the viewer is one for the whole app today (`src/state/gta6_worldview/mod.rs`), where an install's tab and a project's tab need one each | Take | R70b, R70c |
| Shadows, water, sky and an hour of the day | For GTA VI the World screen is lit today from the install's timecycle and weather, casts the sun's shadow in four cascades (`crates/runtime/or-renderer/src/world/shadow.rs`) and draws a sky and water; GTA V's timecycle is parsed today (`crates/io/or-io/src/gta5/data/timecycle.rs`), which NodeRunner does not read | No reader of GTA V's `water.xml`; which parts of the day are GTA VI's alone is not written down; the cost of every material through the game's own shaders over a whole map is unmeasured | Adapt: GTA V's files feed what the World screen has | R70d |
| The map as of a level of DLC | A server pins a build of the game; the mount takes a level to stop at today (`DlcCutoff` in `crates/title/or-title/src/vfs/game_mount.rs`), and `openrage-cli` takes it as `--dlc`, by pack or by build number | Every call of the app mounts every pack, and no index is keyed by the level | Have: the screen passes the level and the index is keyed by it | R70b |
| The map as story mode or as multiplayer | The two load different maps, and a server runs the second | Nothing in the tree makes this choice for GTA V | Take | R70b |
| Maps only a script requests, with the alternatives of one place hidden | Without it two versions of an apartment draw in one place; for GTA VI the World panel lists script-managed groups with a filter today, and `crates/domain/or-world/src/state.rs` says which maps the game requests at an hour and a weather | NodeRunner hides by naming rules, and a rule by name is a guess | Adapt: a list the person toggles, grouped by the manifest's own groups, with a naming rule only as a default the list states | R70f |
| One field that finds a name, a hash, a map, a type, a pasted position or a command | The palette, the name index and Go to File exist today | With a file open the key of Go to File puts the focus in the field of Files, and a model placed many times has no one place to go to | Adapt: one palette with places among its results, listed under the field of Files too; a model's row goes to the nearest placement and steps through the rest | R70e |
| A position typed as two numbers, with its height estimated | A script may print a position as two numbers | Needs the ground under a point | Take | R70e, over R70g |
| Layers for what the game hides: roads, navmesh, scenario points, collision, rooms and portals, occluders, parked vehicles, timecycle zones and far lights | A mapper debugs with them; roads, navmesh, scenario regions, rooms, portals, box occluders and bounds are read today | A map's timecycle modifiers were only counted and its far lights only tested for presence, and a parked vehicle gave a position and one length; the data part of R70o has since typed all three, and `crates/domain/or-world/src/map_data.rs` reads each with every member the installs state; each layer is a draw path to keep fast | Take, with presets named after a task | R70f |
| The levels of detail shown one at a time | A mapper looks at the far city alone | | Take, as layers | R70f |
| A radius for a layer, set in the panel and in the settings | A layer over the whole map costs the frame | | Take | R70f, R70v |
| A box for each object, with a mark where its model is unknown | Shows what is placed where nothing draws | One box would stand both for a model still loading and for an archetype no file answers | Adapt: a box is what an object draws as in both cases, the two differ by their outline and a glyph, and the second is a finding | R70c |
| A ground grid with a key | Gives scale over empty ground; a model's viewport has a grid switch today | The world screen has none | Take, with a cell size | R70v |
| Ambient sound zones and fixed emitters drawn, each sound played | The audio data is read today and a sound container plays | The zones are records of game data that no layer draws | Take | R70f |
| Every sound file of the game listed and heard, with its loop and its stereo pair, and saved as WAV | A sound container opens today with Play and Export WAV (`src/ui/panels/inspector/awc.rs`), found through Files and Go to File | One list of every sound of the install is not built, and none is planned | Have | |
| The types a type file defines, with how many of each are placed and a way to the nearest | Uses and Placed by answer it today for one model | An archetype's row carries no count | Adapt: Show in world on an archetype's row, with its count of placements | R70e |
| A panel of what is loaded | The Statistics panel exists today (`src/ui/panels/statistics.rs`), and for GTA VI the World panel says what streams | | Have | |
| The install found with nothing to set up, and a second install offered | Installs are discovered today (`crates/title/or-title/src/install/discovery.rs`) and chosen under Settings | | Have | |
| Life on scenario points: what a point would spawn, drawn on it | Shows what a point does without starting the game | A simulation of the game's own choice, which nothing here can check | Later | R70t |
| An archive explorer | Files reads and writes the install's archives today | | Have: a row gains Show in world | R70e |
| Camera bookmarks | The GTA VI viewer lists places to go to and copies the camera as a line today | Its places are computed from the index, the densest cells of the maps (`src/ui/panels/world_viewer_places.rs`), and none is saved by a person | Take | R70e |
| GTA IV, its episodes and the first trilogy, read only | Their archives read in `or-io` | The owner asked for GTA V, and [R57b](Roadmap.md#r57b-earlier-rage-titles) holds the earlier titles unscheduled | Leave | |

## Editing objects

Selecting, moving, adding and copying what a map places. The operations are R70h, the tools R70i and placement from a library R70j; [World_Editor_UX_Design.md](World_Editor_UX_Design.md) says what the pointer and the keys do.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| A click selects, Ctrl adds, a second click takes what is behind | CodeWalker and Blender select this way | A pick by the pixel needs an id pass the world renderer lacks, and a ray at boxes picks through walls | Take, by an id pass | R70i |
| A rectangle that selects objects | Mappers move groups; NodeRunner has a rectangle for collision polygons only | Ambiguous through walls | Add: it takes what is visible | R70i |
| A manipulator to move, turn and scale, with steps | Built today: `or_maps::gizmo`, with a grid step, a turn step and a scale step | It draws on the boxes of the Map screen, through a list of lines | Have: drawn in the world, with a choice of the object's axes or the world's | R70i |
| Typed values for position, rotation and scale | Built today, with a key for every handle | | Have | R70i |
| A group moved about its middle as one step | A row of props or the furniture of a room moves together | | Take | R70i |
| Drop to the ground on the real collision, and follow it while moving | The complaint NodeRunner's author lists first; today's ground is the highest box under the object | Needs the world's collision as a query | Take | R70g, R70i |
| Placing by the surface: stand on a floor, hang on a wall | A prop lands where it belongs with no number typed | A rule of thumb about what hangs | Take, with the rule shown and a key to switch it | R70j |
| Copy and paste | | Delete, Copy and Paste have no target for a placed thing today | Take | R70i |
| A duplicate | The inventory shows no general duplicate in NodeRunner | | Add | R70i |
| A duplicate repeated with the same offset | A row of barriers along a street in a few keys | | Add | R70i |
| A snap to another object: the faces and corners of its box, and its pivot | A crate flush against a wall; the snaps built today are a grid step and the ground | A box is not the model's surface, so a snap can leave a gap | Add: on a held key, with the box drawn while it snaps | R70i |
| The model of a selection swapped for another | A small operation over `or_maps::edit` | | Take | R70i |
| A moved game object keeps its far models with it | Otherwise the far model stays where the object was | A far model may sit in another map, and may stand for a neighbour too | Add: the levels that stand for the object alone move, each file named before the step; one shared with a neighbour stays and is named | R70i |
| A new map of the project, which receives what is added | Every map project starts with one | A class written with no donor has a signature word that nothing derives ([R62](Roadmap.md#r62-the-rbm-layout-synthesiser)), and the members an entity states differ by title | Adapt: made from a donor of the mounted install until a build from empty is sound | R70h |
| The first change to a game object makes its map the project's | NodeRunner rewrites the map into the project | Here it is a new door into a project: the map becomes a source with the game's file kept as its base | Adapt: the first edit imports the map, as one step with its toast (ruling 3) | R70h |
| Undo, and a History to click back through | The project's History is kept across sessions today, and a click goes back to any step | The Map screen's Undo is a stack of its own, held in memory and reset with the project (`src/state/workbench/maps.rs`); History keeps a bounded count of steps (`KEPT` in `crates/domain/or-project/src/snapshot.rs`), each the project file whole, and refuses to go back over an import whose source changed since | Adapt: a world step becomes a step of that History and holds its operation and the inverse | R70h |
| A rescue copy every few minutes | | An edit writes its source at once here | Leave | |
| Every written file read back before it replaces the old one | Built today: a build reads every file back, and a source edit refuses what does not compile to itself | | Have | |

## What a project changed

NodeRunner compares a project with the game's files and lists what it changed. R70y builds that list from the base a changed map keeps, shows it in the world and in a panel, and puts one change back.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| The objects a project added, moved and deleted against the game's file, with counts and a filter by kind and by name | A mapper checks a change before shipping it; NodeRunner finds the list by pairing objects with the install's file of the day | No such list exists today; Compare with original compares one file by record and field ([R49e](Roadmap.md#r49e-compare-a-file-with-another-or-with-the-original)) | Adapt: the list is read from the base a changed map keeps, and objects that cannot be told from a twin are listed as undecided and never as deleted | R70y |
| A click on a change selects the object, or goes to where a deleted one stood | The list leads to the place | | Take | R70y |
| The changes shown in the world as well as in the panel | A deleted object draws nothing, so nothing else leads back to it | | Add: a deleted object draws as an outline while its row is selected | R70y |
| One object, or one deleted building, put back as the game has it | A mapper who finds a wall gone weeks later has Undo and History today, and both are bounded | A building comes back with its far models, and with its collision and occluders once R70l and R70o remove them | Add: one step, read from the base | R70y |
| Every change to one map discarded | | | Add: after a question that counts them | R70y |
| A warning when a title update changed the game's file under a changed map | The base a source keeps is then no longer the install's file | Check against the game runs today only when a person asks it of one source | Add: a finding raised when the index is rebuilt, with the merge that carries the changes onto the new file | R70r |

## Removing what the game placed

A deletion of a game object and what follows from it. The chain across files is R70k, the collision R70l and the occluders R70o.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| A deletion that takes the far models and corrects the child counts of their parents | Without it the far model stays; `WorkingSet` in `crates/domain/or-world/src/execute.rs` repairs every link across files today (`executing_a_plan_keeps_every_link`) | It rewrites maps the person did not touch; the working set writes a shrunk array only where a schema snapshot is staged, which a user's machine lacks, and the typed edit of `or-maps` renumbers inside one file | Take, with each file named before it happens, carried over the typed roots | R70k |
| What goes, what stays and why, shown before the deletion | The person confirms a preview and not a question | | Take | R70k |
| The collision of a deleted building removed from the bounds of its area | Without it a wall nobody sees stays | Which polygons belong to a building is a geometric judgement, and a changed bound has no source form today | Take, as a proposal the person edits and confirms, file by file | R70l, over R70w |
| The collision of low slabs and decals kept, with a switch that takes the ground too | A person who digs a basement needs the ground gone | | Take, off by default | R70l |
| The occluders over a deleted building removed | Without it the scenery behind the building disappears | The same judgement; occluders were carried, and the data part of R70o types them | Take, as a proposal of the same kind | R70o |
| A script line that hides a model, as the other way | No file changes, so no conflict with another resource | It runs on every client and removes no collision | Take, as a form of Copy for script | R70k |
| An object that cannot be told from a twin is flagged and left | The person is told which objects were skipped | | Take | R70k |

## Collision

Editing the polygons of a bound. The query that finds what a ray hits is R70g, the form a project keeps a changed bound in is R70w and the tool is R70l.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Polygons picked, deleted, given another material and moved | Opens a passage or removes a wall; a bound is built and rebuilt today (`crates/io/or-io/src/gta5/bounds/build.rs`) | A changed `.ybn` ships whole under its name, a second kind of conflict between resources | Take | R70l |
| A rectangle that takes the visible polygons | | | Take | R70l |
| The collision of a building follows it when it moves | Otherwise the wall stays behind | The same judgement as for a deletion | Take, as an option that takes the polygons inside the building's box, leaves the ground and says what it moved | R70l |
| The three versions of an area's collision kept alike | The game reads one for people, one for vehicles and one for the rest | Three writes for one gesture | Take | R70l |
| Collision and road changes kept by the project | NodeRunner keeps them in a store apart from the entity edits | The source of a `.ybn` is planned and not built, and a `.ynd` has no source form ([layout per format](Source_Format_Design.md#layout-per-format)), while a project holds no compiled game file | Add: a changed bound and a changed path region are sources, each with its base, its cook and its compare | R70w |

## Interiors

The rooms, portals, entity sets and objects of an interior, and its sound. [R47](Roadmap.md#r47-the-plugin-slate-after-the-first) files an interior builder, portals and room bounds, as a plugin, so the rows that edit a room or a portal or make an interior on screen wait for ruling 2. The declarations with their install suite, the entity set switches, placement in a room and the health rules do not wait. The audio is R70x.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Rooms and portals drawn, named on hover | Read today | | Take | R70f |
| Entity sets shown and hidden, with the script line that turns one on | A server turns sets on by script; the sets are read today | A set the name index lacks has only a hash | Take: a set with no name is written as a comment | R70m |
| The records of an interior typed: rooms, portals, entity sets and its timecycle modifiers | Every edit below needs them, and R70f draws them | Typed by the data part of R70m, by the plan's default (ruling 14): declaring them was put to the owner on the sheet of 2026-10-21 and not answered ([rulings in force](Workbench_Plan.md#rulings-in-force)). Its install suite passes on the Legacy and the Enhanced install ([R70m](Roadmap.md#r70m-interiors)) | Add: the data part of R70m, with its install suite, which waits for no screen row | R70m |
| A room's box, name and settings, with the portals on a moved wall following | | It is the interior builder. The operation that states a room's members is built with no screen ([R70m](Roadmap.md#r70m-interiors)); nothing moves a portal with a wall | Waits (ruling 2) | R70m |
| A portal's corners, rooms and flags; a portal fitted to the nearest wall, its corners put in the order the game expects, its two rooms swapped | | The same. The operation that states a portal's rooms, corners and flags is built with no screen; nothing fits a portal or orders its corners | Waits (ruling 2) | R70m |
| A room or a portal added and removed, with the objects and the other portals renumbered | | The same. The operations are built with no screen, with every index renumbered | Waits (ruling 2) | R70m |
| A new interior, a copy under a new name, a second instance | Where an interior starts | The same; and a record made from nothing takes its layout from a donor, as a new map does | Waits (ruling 2) | R70m |
| An object placed in an interior and put in a room, the room chosen automatically | The objects of an interior are entries of its archetype, in its own frame, each attached to a room | The map that receives new objects is not where it goes, and the first edit of a game interior imports its type file | Take: a prop joins the room under the pointer, and builds into the type file and into no map | R70m |
| An object attached to a portal, or filed in an entity set and taken out; an entity set made and renamed | A door closes its portal with it | The plan counts these with the builder. The operations are built with no screen, but for the renaming of a set | Waits (ruling 2) | R70m |
| The health of an interior | The shape of `or_maps::RULES` today | | Take, as rules, [listed below](#the-health-checks-of-an-interior) | R70m |
| The sound passages and room audio of an interior shown, with a notice on a game interior that was moved or copied | A moved interior lets sound through its walls until its occlusion is written | How the game finds an interior's occlusion file is measured on the install before the notice is written | Take, read only | R70m |
| The audio of an interior written at build: the game data of the interior and its rooms, then the occlusion file, then the manifest's lines | Without it sound passes through walls | The game data of an interior and its rooms is added, edited and removed in `or-io`, with no verb and no screen ([R70x](Roadmap.md#r70x-the-audio-of-an-interior)); the members of the occlusion metadata are hashes without a schema snapshot | Take, in that order, the occlusion file through a donor's type table | R70x |
| The occlusion file written through a root typed by hand | Names in the place of hashes | A typed root outside the families the record names | Waits (ruling 16, a typed root declared by hand) | R70x |

### The health checks of an interior

Each check is a rule of the Problems panel in R70m. The second column says what `crates/domain/or-maps/src/rules.rs` and `crates/domain/or-maps/src/interior.rs` hold today. Four rules of the second file are no row here: a portal with one room on both sides, a room that counts other portals than name it, an entity of a set in a room the interior does not hold, and an entity attached to two rooms.

| Check | In `or-maps` today | Verdict | Row |
|---|---|---|---|
| A portal names a room the interior lacks | `interior-broken` | Have | |
| A portal has other than four corners | `interior-broken` | Have | |
| A portal has no area, is not flat or is twisted | None | Take | R70m |
| Two portals are alike | None | Take | R70m |
| A room or a portal attaches an object past the interior's list | `interior-broken` | Have | |
| An object is in no room | `interior-entity-unattached`, for one in no room and on no portal: an object the installs attach to a portal is in no room | Have | |
| An object's model is held by nothing | `archetype-missing`, for an entity a map places | Take, for the objects of an interior | R70m |
| A room has no portal | `interior-room-unreachable`, for a room no chain of portals joins to the outside, asked of an interior that has a portal to the outside | Have | |
| An interior has no portal to the outside | `interior-sealed`, a note: the installs hold such interiors | Have | |
| A room's box is empty, or far larger than its objects | None | Take | R70m |
| An entity set is empty, or two sets have one name | None | Take | R70m |
| The two arrays of an entity set differ in length | `interior-broken` | Have | |
| An interior has more rooms than the room number of a collision polygon can tell apart | None | Take | R70m |

## Doors

Finding the doors of a building and saying what a script needs of them, in R70n.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Every door of a building found, paired when double | A server locks doors by script | NodeRunner knows a door by its model's name, and a name the index lacks is a door missed | Adapt: first by the door extension an archetype carries, typed today (`crates/domain/or-world/src/extension.rs`), then by name, saying how many models had no name | R70n |
| Copy for a door script, in the forms servers use | A script developer asks for it | The forms belong to other projects and change | Take, as templates kept as data (ruling 10) | R70n |
| The arc a door opens through, and whether furniture blocks it | Catches a door that cannot open | A reading of the game's tuning | Take | R70n |
| A door opened and closed in the view | The arc is judged in its place | | Take: on a double click, stopping where its arc is blocked | R70n |
| The zone that makes a door open by itself, drawn, and a door's tuning read | Says why a door opens for one person and not another | Read only | Take | R70n |
| A door replaced by another fitted to its frame | Saves placing and turning the new door by hand | | Take | R70n |

## What a map holds besides objects

Parked vehicles, zones, lights and occluders are R70o, and roads, scenario points and the navmesh R70p. The data part of R70o, the declarations in `or-io` with their install suite, waits for no screen row.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Parked vehicles: place, turn, size, model and colours | The record is in the map | Only a position and one length were read; the data part of R70o types every member the installs state, with `flags` stated whole; flags nobody names | Take, with a bit without a name shown as a bit | R70o |
| A vehicle of the game drawn on a selected parking spot | The size and the heading are judged by eye | | Take | R70o |
| Timecycle zones: place, size, name, strength and hours | The record is in the map | Only counted before; the data part of R70o types the seven members the installs state | Take | R70o |
| Night lights seen from afar: place, colour, strength and hours | The record is in the map | Only tested for presence before; the data part of R70o types both families, and a save of the pair refuses one out of step; a pair of maps that must stay in step | Take | R70o |
| Occlusion boxes placed and sized, turning about the vertical only | Fixes the scenery showing through | The manipulator needs a limit for each kind of record; the data part of R70o types the eight numbers of a box | Take | R70o |
| Occlusion meshes shown and removed, as boxes are | A mesh hides as a box does | Only a mesh's stored box was read; the data part of R70o types the mesh, its bytes as one run that `dataSize` counts | Take | R70f, R70o |
| Boxes fitted to a building, and for an interior thin boxes in its outer walls with its openings left free | Saves drawing each box by hand | The fit is a heuristic over collision | Take, as a proposal the person accepts | R70o |
| A preview of what the occluders hide | The world viewer has an occluder buffer today | | Take | R70f |
| Roads: a node's speed and flags, its lanes, nodes and links added and removed | `.ynd` is owned and saved today | An area file is rewritten whole and has no source form; a link names its far node by region and index, so renumbering one region breaks its neighbours' links; junctions are kept and not made | Take: a node another region names is disabled and not deleted | R70p, over R70w |
| Scenario points: move, type, hours, model set and its other fields; add and delete | The region is typed today | A region made from nothing waits on [R62](Roadmap.md#r62-the-rbm-layout-synthesiser) | Take, in regions that exist | R70p |
| The navmesh seen, with warnings where a change leaves it wrong | Drawn today | No writer of new navmesh, here or in NodeRunner | Take the warnings; authoring is left out | R70p |

## The model workshop

Small changes to a model with no round trip through Blender, in R70s. OpenRage's model is a source of glTF and JSON, so each is an operation on that source.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| A part of a model removed, moved or copied; a box or a plane added | Small fixes made in place | Each is an edit of the model's source, in every level of detail | Adapt: operations on the model source | R70s |
| The parts of a model listed, each hidden or shown alone | The list a removal chooses from | | Take | R70s |
| How many times a model is placed, said before it is edited | An edit of a game model changes every place the game puts it | | Take | R70s |
| An edited game model saved as a new prop, with only the object it was opened from swapped to it | A new name changes one place and leaves the game's model everywhere else | It needs a type file that holds a copy of the game's archetype under the new name | Take | R70s |
| A part taken from one model and added to another | | | Take | R70s |
| The neighbouring objects drawn around the model | A cut is judged in its place | | Take | R70s |
| A texture replaced, exported and resized | Built today in the Texture workspace | | Have | |
| The texture slots of a model listed, with those not found | The viewport's texture badge lists them today | | Have | |
| A prop made from an image | A sign or a badge made without a modelling tool | It is a drawable made from nothing, which [Mesh_Interchange.md](../features/Mesh_Interchange.md#what-this-does-not-do) lists as not done | Waits (ruling 13, author mode) | R70s |
| A door cut into a wall, and a building opened with a room behind it | An interior behind a front the game left closed | Experimental in NodeRunner; it touches the model, three collisions, the occluders and a new interior at once | Later, last and marked experimental | R70s |

## The browser of models and vehicles

NodeRunner's model viewer is a browser of every object type and every vehicle of the game. R70z builds it over the Library of R70j. A list by name does not wait; its grid of thumbnails is the thumbnail browser of ruling 2.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Every object type and every vehicle browsed in one place | How a mapper and a script developer find a model | A vehicle's class and pack come from the install's vehicle data, which `crates/title/or-title/src/install/vehicles.rs` reads as the file's text states them, and the two lists are paged with no screen ([R70z](Roadmap.md#r70z-a-browser-of-models-and-vehicles)) | Take | R70z |
| Thumbnails of what is browsed | The thumbnail cache exists today (`src/render/thumbnails.rs`) | R47 files a thumbnail browser as a plugin | Waits (ruling 2) | R70j, R70z |
| Filters and order by class, by pack and by size | | The class and the pack are read from the same vehicle data | Take | R70z |
| Favourites, the models opened last, and back and forward | | | Take | R70z |
| The view of a model saved as an image: a transparent or a keying background, a chosen size, preset views and a narrow lens | Pictures of a server's props and vehicles | The app saves a texture as an image today and no view | Take | R70z |
| The places a shown model stands, nearest first | Placed by answers today which maps place a model | It gives maps and no order by distance | Adapt: Placed by, ordered by distance, with Show in world | R70z |
| A vehicle drawn in the colour combinations the game plans for it, with a random pick and the game's palette | A parked vehicle's colours are numbers of the same palette | The install's colour and variation data is read by nothing today | Take | R70z |
| A vehicle's sheet, from the game's vehicle and handling data | | The same | Take | R70z |
| The lines that paint a vehicle in the colours shown, copied | | | Take, as a form of Copy for script | R70z |

## People and scenes

Peds, cutscenes and a character that walks, in R70t, which lands in the parts its brief lists.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Every ped with its outfits and every clip | The Animation workspace and the dressed ped of the Wardrobe do most of it today | A browser of peds is new | Take | R70t |
| A ped's model name and the lines that play the clip shown, copied | | | Take, as a form of Copy for script | R70t |
| A cutscene played in its place with its camera | A cutscene reads as metadata today | A player of its tracks is new, and has no sound | Later | R70t |
| Walking the map: run, jump, climb and swim, stopped by walls | Finds a blocked path before a build | It is the walk mode, and a character's motion written here is not the game's | Waits (ruling 2): then a late part, labelled on screen as a check of the path and not as the game's own movement | R70t |

## What comes in, and what goes out

The library of props and the doors a mapping enters by are R70j.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| A library of props shared by projects, with favourites | Links between projects exist today | A library would be a second kind of place beside the project | Adapt: a library is a project the others link | R70j |
| The game's own props by category | Props are found without knowing a name | A shipped list of names is game content | Adapt: categories derived from the install by rule (ruling 11) | R70j |
| A mapping imported from a folder, a zip or a package | Adopting a folder exists today, and `openrage-cli project adopt --into` takes a folder or a zip and reports it first | Another tool's `.oiv` is a new door ([R50](Roadmap.md#r50-package-installation)), and no screen shows the report | Take the folder and the zip; the package comes with R50 | R70j |
| The XML a Blender add-on or CodeWalker writes, converted on import | How props and interiors leave Blender today; without it a prop arrives as glTF, with no collision and no type file | The ruling of 2026-09-22 that this XML is no compatibility target, which [Workbench_Plan.md](Workbench_Plan.md#held-back) holds back; a format with no specification | Waits (ruling 5): read on import, landing as sources, and never stored | R70j |
| A watched folder that takes each new export | The loop a modeller works in; a file watcher exists today (`src/state/file_watcher.rs`) | It takes what the row above reads | Waits (ruling 5), with the row above | R70j |
| A game file written out as that XML, with its textures as DDS, to open in Blender | How a mapper takes a game building into Blender with the add-on they have | Reading lands as sources and keeps no XML; writing makes the XML an output OpenRage must keep compatible, which reopens the ruling of 2026-09-22 more widely than reading does; `openrage-cli mesh export` writes glTF, which carries no collision and no type file | Leave | |
| A report after each import of what does not find what | The import report panel and the rules exist today | | Have: the report gains the map rules | R70j |
| Files locked by a server's asset escrow recognised, kept whole and drawn as a box | A build no longer fails on them; the Wardrobe's import names such a file today and leaves it out (`crates/domain/or-wardrobe/src/import.rs`) | They are never decrypted | Take | R70j |

## Projects and shipping

A project, its build and the server it is built for. The build is R70q and the server's other resources R70r.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| One project for a resource | A project holds several contents and declares several builds today | | Have | |
| Several projects open together in one world, one of them receiving what is added | A second project joins this one as linked content, a layer of the world's stack: it is drawn in the same world and read into this project's builds | One project a window is a ruling, and linked content has one writer, the project that holds it | Adapt: the other project is linked and edited in its own window; new objects go to this project's receiving map | R70c |
| A FiveM resource in one action | Built today: Ctrl+B | It lacks the manifest of its maps and the line that declares the resource a map | Have: R70q completes it | R70q |
| The line to add to the server's configuration, shown after a build | | | Take | R70q |
| Extents, links and counts recomputed at each write | Built today for extents and links | The streaming extents of a game map grow and never shrink | Have | R70q |
| The server's resources drawn in the world | What a mapper sees is then what the server loads | A server's folder is a new place to read (ruling 4), and a new layer of the mount | Add: the folder mounted as a read-only layer, in the order the server starts its resources | R70r |
| Another resource of the server that ships the same file is named | Two resources that ship one map are a conflict a server cannot load around | NodeRunner names it at export | Adapt: a finding raised by the edit that imports the file, and repeated before a build writes | R70r |
| Its changes taken into this project by a three-way merge | Ends the conflict for a map | NodeRunner's base is whatever the install holds that day | Adapt: the base is the game's file the source keeps | R70r |
| A conflict on a file that is not a map: a bound, a road file, an occlusion map | A changed bound ships whole under its name | No merge has a form for them | Take: named and not merged, with the offer to leave the file out or go on, until the compare of R70w gives a merge a form | R70r, R70w |
| A conflict with the person's other projects | | | Adapt: the scan reads any folder of built resources, so the builds of a person's other projects are compared as a server's are | R70r |
| Any resource folder checked as the server will read it | Useful with no project | | Take, as a verb of `openrage-cli` first: [the checks](#the-checks-of-a-resource-as-a-server-reads-it) | R70r |
| A story pack | Built today: a Map content is laid out as the packs of two installs were measured to keep theirs, its type files and models ship, and a content with a map is still locked, for the change set that turns maps on, which no pack of the installs shows for the story (`the_cell_of_a_singleplayer_dlc_says_yes_to_type_files_and_models_and_no_to_a_map_for_what_turns_it_on`) | The game streams a pack's maps from a nested archive, and the Map content hands its maps over at the stream folder; the nested map archive was put to the owner on the sheet of 2026-10-21 and not answered (ruling 15) | Take: the Map content hands its maps over inside the archive, and the cell unlocks | R70q |
| After a story build, the line the DLC list needs, to copy | Built today: the Build tab states it (`src/ui/panels/project/build_part.rs`) | | Have | |
| Whether the install reads a mods folder at all, with the steps when it does not | Built today: Settings reports the loader and what is missing (`src/ui/panels/settings_overlay_loader.rs`) | | Have | |
| A mods copy of an archive that the game has since updated | Built today: found when an install opens (`src/state/overlay_refresh.rs`) | | Have | |
| The pack installed into the mods folder with the DLC list edited, refused while the game runs; the packs installed listed, and one removed | One action to test in story mode | It writes into the install; installing an `.oiv` was left undecided on 2026-11-21 | Waits (ruling 7): the build writes the pack and says where it goes | |
| A world shown from an install of one edition and a build for the other | An Enhanced install ships what a Legacy server cannot load as it is | A build converts a model and a texture to its target and ships a cooked map as it is, and the members the world family states differ by title | Add: models and textures convert at the build; a map taken from the other title's install is refused for editing, with a sentence | R70h |
| A project shared through a repository | Sources are text Git compares today | A changed game map is a source taken from the game: the project's managed ignore block leaves it out and Check before sharing lists it ([commit policy](Source_Format_Design.md#commit-policy)), so a clone lacks every changed game map | Have: the commit policy stands, and the toast of the first edit and the guide name the setting a team turns on (ruling 17) | R70i, R70v |
| Positions and names copied in the form a script takes | A script takes a position, a name or a hash in a form of its own | | Take: [the forms](#copy-for-script) | R70e |

## The checks before a build

NodeRunner checks a project before it exports, each finding with a sentence and a way to the place. Here each is a rule of the Problems panel, run while the person works, and a rule with a safe fix offers it. The second column says what `crates/domain/or-maps/src/rules.rs` holds today; the rules it holds that the inventory does not show, among them `hierarchy-broken`, `extents-short` and `entity-twice`, stay as they are. [The health checks of an interior](#the-health-checks-of-an-interior) are listed with the interiors.

| Check | In `or-maps` today | Verdict | Row |
|---|---|---|---|
| Several objects state one guid, with a repair in one action | `guid-twice`, with no fix | Take: the safe fix, a new guid for each copy | R70i |
| An object's model is found nowhere, perhaps because it belongs to a newer build of the game | `archetype-missing` and `archetype-undeclared` | Have |  |
| An object uses a model that only another open project ships | None | Adapt: it does not arise, since linked content is read into this project's builds |  |
| An occlusion box reaches outside the building it stands in | None | Take | R70o |
| An occlusion box is crossed by a road or a pavement | None | Take | R70o |
| An occlusion box overlaps an interior | None | Take | R70o |
| An occlusion box is too small to hide anything | None | Take | R70o |
| An occlusion box lies mostly inside another | None | Take | R70o |
| A road node has no link | `node-unreached`, in `or_maps::roads::rules`, a node no link reaches | Take | R70p |
| A link goes to a node that does not exist | `link-node-missing`, in `or_maps::roads::rules` | Take | R70p |
| A link has no link back | `link-one-way`, in `or_maps::roads::rules` | Take | R70p |
| A vehicle link has no lane either way | `link-without-lane` is listed as not checked: a link's word names no lane | Take | R70p |
| A road node is linked to a footpath node | `link-road-to-footpath`, in `or_maps::roads::rules` | Take | R70p |
| A road node or a scenario point floats above the ground, or lies under a surface | `node-in-air` and `node-buried` for a road node, against a ground no caller hands them yet | Take | R70p, over R70g |
| A scenario point is left on a deleted object | None | Take | R70p |
| A building is added on walkable ground | None | Take | R70p |
| A deleted building leaves its floors in the navmesh | None | Take | R70p |
| A door model of the project will not open, because its archetype lacks the flag the game's doors carry | None | Take: the flag is measured on the install first | R70n |
| A game door was moved or deleted, and a lock script finds a door by its model and its position | None | Take | R70n |
| Two doors stand at one place | None | Take | R70n |
| A door does not open wide enough, because its arc is blocked | None | Take | R70n |
| A door is left where a building was deleted | None | Take | R70k |
| A game interior was moved or copied and its sound occlusion no longer matches | None | Take: R70m says so and R70x writes the occlusion | R70m, R70x |

## The checks of a resource as a server reads it

NodeRunner runs these after every export and on any folder a person names. R70r builds them as a verb of `openrage-cli` that checks a folder with or without a project, and the build runs the same findings on what it wrote. The second column says what the tree holds today.

| Check | In the tree today | Verdict | Row |
|---|---|---|---|
| The resource has no manifest | A build writes `fxmanifest.lua` unless the project holds its own; a folder from elsewhere is not checked | Take | R70r |
| The manifest has no version line, without which a server rejects the resource | None | Take | R70r |
| The resource holds maps and no line that declares it a map | None | Take: R70q writes the line for the build of a Maps content, and a build of Files content that ships a map writes none, which R70r's check warns of | R70q, R70r |
| A file that the manifest or one of its data lines names is absent | None | Take | R70r |
| One file name is held more than once under the stream folders, of which the server loads one | `file-twice` is another case: two sources of one project that cook to one file | Take | R70r |
| A file is of a kind the server does not stream | None | Take | R70r |
| A file's streaming memory passes the size at which the server warns | The build's `memory` limit weighs a whole build (`MEMORY_LIMIT` in `crates/domain/or-project/src/build.rs`) and no single file | Take: the size is the one the server's own warning names, and the rule cites where it was read | R70r |
| A type names a texture dictionary that neither the resource nor the game holds | None | Take | R70r |
| A type has no model | `archetype-without-model` | Have: run on the folder | R70r |
| A model is described by no type file | `model-unused` | Have: run on the folder | R70r |
| An object's model is described by no type file | `archetype-missing` and `archetype-undeclared` | Have: run on the folder | R70r |
| A file does not read, or an asset escrow locks it | `file-unreadable`, for a source of the project | Take, with the recognition of R70j | R70j, R70r |
| The build's folder lies under a folder of the server's resources that is not a category in brackets, so the server never starts the resource | None | Take | R70r |
| An earlier build of the same resource lies elsewhere under the server's folder | None | Take | R70r |
| Another resource of the server ships a file of this one | None | Adapt: a finding from the edit that imports the file, repeated before a build writes | R70r |

## Copy for script

NodeRunner copies what a script needs in the form the script takes, every copy following one chosen default form. R70e builds the forms as templates kept as data, in Lua, JSON and C#, and the rows that own a kind of thing add its forms.

| Form | In the tree today | Verdict | Row |
|---|---|---|---|
| A position | None | Take | R70e |
| A position with its heading, also from a right click | None | Take | R70e |
| A list of a selection, as name, position and heading | None | Take | R70e |
| A model's name, a map's name, the path in the archives | Copy name and Copy path on a file's row (`src/ui/commands/files.rs`) | Have: R70e offers them on an object in the world | R70e |
| A hash in hexadecimal and as the signed number a script's hash function returns | Copy hash on a file's row | Take: both spellings | R70e |
| The lines that spawn an object or a vehicle | None | Take | R70e |
| The line that hides a model | None | Take | R70k |
| The lines that turn an entity set on, a set with no name written as a comment | None | Take | R70m |
| The door tuning entry, the line that gives it to a model, and the forms door scripts take | None | Take, as templates kept as data | R70n |
| The centre of a zone | None | Take | R70o |
| A ped's model name and the lines that play a clip | None | Take | R70t |
| The lines that paint a vehicle | None | Take | R70z |
| The line for the server's configuration | None | Take | R70q |
| The line for the DLC list | The Build tab states it | Have | |
| One default form for every copy | None | Adapt: the button keeps the last form used and lists the others | R70e |
| A pasted position read in any of these forms | None | Take | R70e |

## Working together

Several people in one project, in R70u, which lands in three parts: the merge of R70r wrapped as the driver Git calls for a map source, a session on a direct connection, and a relay. Nothing in OpenRage reaches the network unless a person asks: the one client in the tree fetches the natives lists on request (`crates/tools/or-cli/Cargo.toml`). A session reaches it when a person starts one, and a relay is a program a team runs for itself (ruling 6).

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Several people in one project, each seeing the others' edits, selection and camera | A team maps together | A relay is a service somebody runs | Later: the host orders the operations of R70h and sends them | R70u |
| The host admits or refuses each person who asks to join | | | Later | R70u |
| What a person holds selected is locked to the others, and a tie goes to one of them with a line that says so | Without it two mappers undo each other | | Later | R70u |
| A guest whose program or game files differ from the host's is refused with the reason | Both must read the same game | NodeRunner compares versions and files | Later: the stacks are compared, edition, level of DLC, server and linked content, and the refusal says what differs | R70u |
| Only the host saves and exports | One writer of the sources | | Later: only the host builds | R70u |
| A person goes to another's view | | | Later | R70u |
| A person's own Undo and Redo, carried out by the host | One order of steps for everyone | | Later | R70u |
| A guest who lost the session joins again and is brought up to date | | | Later: a person who joins late has the same sources byte for byte | R70u |
| A ceiling on how many people join | | No figure of NodeRunner's is repeated | Later: the host sets it | R70u |
| A map merged by object when two people share a repository | Sources are text, and a map's XML merges badly by line | It merges a changed game map only where the repository holds the sources taken from the game | Add: the merge of R70r, wrapped as the driver Git calls for a map source | R70u |

## The shell

Keys, hints, settings and the windows around the world. [World_Editor_UX_Design.md](World_Editor_UX_Design.md) says what each looks like; the keys, the settings page and the guide are R70v.

| Capability | For | Against | Verdict | Row |
|---|---|---|---|---|
| Keys that can be changed | Built today: a binding is changed, cleared and reset in Settings and kept in the preferences (`src/ui/shortcuts.rs`) | A world tool needs a key that means something else on another screen | Have: a conflict is one key for two commands read in one place (`Place` in `src/ui/commands/table.rs`) | R70v |
| Presets of keys, with one that matches CodeWalker's | Presets were left open on 2026-10-03; a mapper who comes from CodeWalker keeps the same keys | Settings offers presets today (`src/ui/shortcuts/presets.rs`), and CodeWalker's holds one binding until its tools have commands | Take | R70v |
| A bar that says at every moment what the pointer and the keys do for the tool in hand | How the keys are learnt | The status bar has a hint only for a window with nothing open today (`src/ui/panels/status_bar.rs`) | Take: the hints change while a handle is dragged and while the camera flies, and the bar gives the camera's position and the edition | R70i |
| A right click on an object opens a short menu: go to, copy the position, delete, edit the model | A right click in the viewport opens a menu today, and every menu draws from the command registry (`every_context_menu_draws_from_the_registry`) | The right button also flies the camera | Take: a click released where it was pressed opens the menu, and held it flies | R70i |
| A double click on an object opens its model, and on a door opens and closes it | | | Take | R70i, R70n, R70s |
| A game term explained in one sentence when its title is hovered | | | Adapt: Details says an object in a mapper's words first, with the engine's names behind Show internals, and a panel's title carries the sentence as its hover | R70v |
| A welcome with first steps | The start page and the tour exist today (`src/ui/panels/start_page.rs`, `src/ui/panels/tour.rs`) | Neither speaks of the world | Have: R70v adds a tour of the world and a Map template that opens on it | R70v |
| A list of errors, and a report to copy into a bug message | Built today: the Log panel copies its lines and names the crash reports folder (`src/ui/panels/log.rs`) | | Have | |
| Settings for quality, distance, the frame and the camera | The viewer keeps five values today | Most of the settings a mapper changes have no control | Take: [page by page below](#the-settings-page-by-page) | R70c, R70d, R70v |
| Two interface languages | Every catalog of `src/i18n/languages` | The sentences of the map rules are English today, as a library crate's are, so a finding reads in English on a French screen | Have: R70v puts the rules' sentences in the catalogs | R70v |
| A notice that a newer version exists | | It would reach the network unasked | Leave | |
| An updater that replaces the program | | OpenRage downloads and installs nothing on its own | Leave | |
| Translucent windows | | It changes nothing a mapper does | Leave | |
| A panel, or the workshop, dragged out to a window of its own on another screen | A mapper with two screens keeps a model or Details beside the world | A panel floats inside the window today (`src/ui/dock_commands.rs`); a window of the app is a project's window, one for each project, so a panel in a window of its own is a new kind of window with a surface and a share of the viewer of its own | Leave: one window stretched over two screens, with panels floated in it, is what exists | |

### The settings, page by page

What NodeRunner's settings hold, against what OpenRage keeps today. The World screen's preferences are an hour, a weather, the modes, a scale of the levels of detail and a speed (`src/ui/panels/world_viewer_prefs.rs`).

| Setting | In the tree today | Verdict | Row |
|---|---|---|---|
| Quality presets for a modest and for a strong machine | None | Take | R70c |
| A scale of the distance at which detail is drawn | The scale of the levels of detail | Have | R70c |
| Reflections on water | None; GTA V's water is not drawn | Adapt: part of the preset for a modest machine, with no switch of its own | R70c, R70d |
| The distance of fog | The install's timecycle sets the fog | Adapt: the hour and the weather decide it, as in the game, and no setting does | R70d |
| The distance shadows reach | None | Take | R70d |
| Texture filtering | A material's filter is the one its sampler states (`crates/runtime/or-renderer/src/game_shaders/stand_in.rs`) | Leave: no setting overrides the game's own | |
| Which graphics card draws | The app asks for the high-performance one (`src/app/gpu.rs`) | Leave: it stays the app's | |
| Vertical sync | On, set by the app (`src/app/gpu.rs`) | Leave: it stays the app's | |
| The video memory kept for the map | The budgets of the texture caches are preferences (`texture_cache_mib`, `gpu_texture_mib`) | Have | |
| A frame cap, and a lower one while the window is in the background | None | Take | R70c |
| Mouse sensitivity | None | Take | R70v |
| Field of view | Fixed in the viewer, and set by a pasted camera line | Take | R70v |
| An inverted vertical axis | None | Take | R70v |
| Movement letters for an AZERTY or a QWERTY keyboard | The GTA VI viewer reads both sets of letters | Have: the mapper's page gains the choice | R70v |
| The snap steps for move, turn and scale | The Map screen keeps a step of position and of turn, and the manipulator a step of scale | Have: shown in the tool bar of the world, remembered, and on the mapper's page | R70i, R70v |
| The default form of a copy | None | Adapt: the last form used | R70e |
| The ground grid's cell size and colour | A grid switch for a model's viewport | Take the cell size | R70v |
| The radius of the collision, navmesh and object box layers | None | Take, in the Layers panel and on the page | R70f, R70v |
| Interface size and accent colour | Preferences today (`ui_scale_percent` and `accent` in `src/state/prefs.rs`) | Have | |
| The folders of the projects and of the library | A project is a folder the person chooses, and a library is a project the others link | Adapt: no folder is set once for all | |
| The keys | A page of Settings | Have: with presets | R70v |
| Restore defaults | For the keys | Take, for the mapper's page | R70v |

## Left out, and why

Every row above whose verdict is Leave, the one list of sounds that a Have row does not build, and what the plan's summary also names. A sentence from the owner reverses any of them, and the plan asks a ruling on the list (ruling 12).

- **GTA IV, its episodes and the first trilogy.** The request is GTA V, and [R57b](Roadmap.md#r57b-earlier-rage-titles) keeps the earlier titles a future goal.
- **An updater, and a notice that a newer version exists.** The first downloads and installs on its own, and the second reaches the network unasked.
- **A rescue copy.** An edit writes its source as it is made, so nothing is unsaved.
- **Writing the XML a Blender add-on reads.** Reading it on import waits for ruling 5 and stores no XML. Writing it makes that XML an output OpenRage keeps compatible, which is what the ruling of 2026-09-22 refused, and a model leaves OpenRage as glTF ([Mesh_Interchange.md](../features/Mesh_Interchange.md)).
- **One list of every sound of the install.** A sound container opens, plays and exports from Files, and nothing a mapper does in the world needs the list.
- **A panel as a window of its own on another screen, and translucent windows.** A window of the app is a project's window. A panel floats inside it, and the window stretches over two screens.
- **The choice of graphics card, vertical sync and a texture filter of the person's.** The app chooses the first two, and a material keeps the filter the game states.
- **Authoring navmesh, grass, an entity's extensions on screen and a cinematic camera.** NodeRunner has none either. Grass and an entity's extensions are edited as data in the Data workspace, and no navmesh is authored.
- **A link to a running game.** It is not a capability of NodeRunner's, and it stays a non-goal of [the roadmap](Roadmap.md#non-goals): the walk mode runs no game.
