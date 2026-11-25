# World editor UX design

What a mapper would meet in the GTA V world editor, flow by flow: the window, the camera and the keys, each tool, and what the screen says when it waits or refuses.

Status: proposal of 2026-11-25; nothing in it is in the tree except where a sentence says today.

It describes what a person would meet once the rows of R70 are built. [World_Editor_Plan.md](World_Editor_Plan.md) owns the verdict, the order and the briefs, [World_Editor_Parity.md](World_Editor_Parity.md) what NodeRunner Studio does, [the roadmap](Roadmap.md) each row's check, and [Workbench_UX_Design.md](Workbench_UX_Design.md) what the window draws today.

Each subsection names in parentheses the row that builds it, and that row settles the words of a control: this document says object where the Map screen says entity today, and a sentence in italics is a sketch, not a catalog entry. A control that waits for the owner names its ruling by subject, and [the plan](World_Editor_Plan.md#rulings-asked-of-the-owner) lists the rulings.

## The shell the world sits in

### What the window is today

- Today the rail at the left edge has Home, Game, World, Project and Automate (`src/ui/panels/rail.rs`). World is on it only while a GTA VI folder is set, and it opens a workspace, not a tab: the view with the World panel beside it and nothing docked under it (`world_layout` in `src/ui/dock.rs`). The app holds one viewer today (`src/state/gta6_worldview/mod.rs`).
- A project's screens are today tabs before the tabs of its files, Project first. A bar under the tabs offers a file's primary command, and a screen of the project has none ([Switch view](Workbench_UX_Design.md#switch-view)).
- One panel, Details, shows today what is selected ([R69g](Roadmap.md#r69g-one-details-panel)), and the window has one Build control, at the right of the menu bar ([R69c](Roadmap.md#r69c-one-build-control-and-a-save-that-offers-what-can-happen)).
- The Map screen draws today a box for each entity ([what the Map screen builds](Workbench_UX_Design.md#what-the-map-screen-builds)).

### One world, two doors (R70c)

1. The World room is on the rail for a GTA V install too. It opens a World tab on the install, read only and with no project.
2. With a GTA VI folder and a GTA V install both set, the room opens the world it showed last, and the first part of the stack line names the game and changes it.
3. A GTA VI world keeps its World panel as [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md#opening-it) describes it. The Outline, Layers and Details of this document are a GTA V world's.
4. The Map content of a project opens the same viewport as a Map tab, which draws the project over the install and whose tools write. A scanned GTA VI map body keeps the box scene it has today.
5. The viewer becomes one for each world tab, so a World tab and a Map tab show two stacks in one session.
6. With a project open the World tab stays read only. A tool that would write there says so in one line and offers Edit in the project, which brings the Map tab forward at the same camera with the same object selected, and lists the contents when the project holds several. A project with no Map content is offered the action the Map screen's empty state has today.

### The window (R70c, R70i)

A sketch of a Map tab, not a screenshot: no such screen is drawn today. The column of letters is the rail.

```text
+--------------------------------------------------------------------------------------------------+
| File  Edit  View  Project  Automate  Help                                     [Build: harbour v] |
+---+----------------------------------------------------------------------------------------------+
| H | [Project] [harbour: Map] | [prop_bench.ydr x]                                                |
| G | GTA V Legacy . all packs . multiplayer . server: none . project: harbour . new: harbour_a    |
| W +---------------+----------------------------------------------------+-------------------------+
| P | Outline       | [1 Select] [2 Move] [3 Turn] [4 Scale]             | Details | Plan          |
| A |  v harbour    | [Grid] [Turns] [On the ground] [Objects] [Axes]    |  <model name>           |
|   |  > game maps  |        the world, with the manipulator             |  position   x  y  z     |
| ? | Layers        |        on what is selected                         |  [Copy for script v]    |
| S |  [x] Objects  | 12:00 . Clear sky             x <n>  y <n>  z <n>  |  Uses . Placed by       |
|   +---------------+----------------------------------------------------+-------------------------+
|   | Problems 3 | Build | Changes | History | Library | Jobs | Log                                |
+---+----------------------------------------------------------------------------------------------+
```

- **The shell is kept**: the rail at the left edge, the Project tab first, one Details and one Build control. The Outline is the Maps list of today grown to the world, and the Plan is a tab behind Details.
- **The bottom strip** gains Changes (R70y) and Library (R70j). A World tab on an install has no strip: Jobs and Log are tabs behind Details there, as they stand behind the World panel today.
- **The tools** appear with the row that builds each: four with R70i, the collision tool with R70l. The hour and the weather under the view are R70d's.
- **The stack line is a bar under the tabs**, which a screen of a project does not have today; R70c changes that rule for a world. The table that gives each kind of tab one primary command ([R69f](Roadmap.md#r69f-a-rail-of-rooms-and-one-primary-action-a-tab)) gains the World kind, whose command is Edit in a project…. A Map tab keeps Build as its primary action.

### The smallest window (R70c)

The smallest window is today `SMALLEST` in `src/ui/the_builds_read_plainly_on_screen.rs`, and the French screens are held to it by `the_french_screens_clip_break_and_overflow_nothing_the_english_ones_do_not_at_the_smallest_window`, which R70c extends to the world. At that size:

- the bottom strip is closed to its row of tabs until one is clicked, and Layers is a tab behind the Outline;
- the stack line shows the edition, the project and the map that receives, and folds the rest under one button whose hover says the whole line;
- the tools are glyphs with their names and keys in the hover;
- Maximize and the clean frame, which exist today, give the view the whole window.

### The stack line (R70b, R70c, R70r)

Each part of the line is a button that changes it.

| Part | Says | Row |
|---|---|---|
| Game | The game and the edition its files state | R70c |
| Packs | All packs, or the pack the world stops at. The mount takes that cutoff today (`DlcCutoff` in `crates/title/or-title/src/vfs/game_mount.rs`, `--dlc` of `openrage-cli`), and the app always mounts every pack | R70b |
| Mode | Story or multiplayer, a choice nothing has today | R70b |
| Server | The resources folder read as a layer, or *server: none*; absent until R70r merges | R70r |
| Project | The project, and the content when it holds several | R70c |
| Receives | The map a new object goes to, or the interior and the room while the camera is inside one | R70h, R70m |

When the install shown is not of the edition the project's build targets, the line says so beside the game. A build today converts models and textures to its target and ships a map as it is, so [the first edit](#the-first-edit-of-a-game-object-r70h) of a game object is then refused.

## The first visit (R70b, R70c)

1. The start page has a tile that opens the map of the install in use, and the rail has World.
2. A first visit builds the index as a job with progress and Stop. Until the index is held the view is a plain backdrop that says so in its middle, as the GTA VI screen's is today: a frame with no index would hold water and bare ground and read as the world.
3. The world then appears, far models first, and the corner of the view says what is still loading, as it does today for GTA VI. A later visit opens where the person last was, and the index is built again only when an archive changed.

### Waiting and empty states, and refusals

Each state is a sentence and one action, in the place the person is looking.

| State | Where it is said | The action offered |
|---|---|---|
| No GTA V folder is set, or its keys were not read | The middle of the view | Settings ▸ Games… |
| The index is being built | The middle of the view, with its progress | Stop |
| The index was stopped, or failed with its reason | The middle of the view | Build the index again |
| A model is still arriving | A box with a broken outline and a clock glyph | None |
| No file answers an object's model | A box with a full outline and a question glyph; in a project, a Problem | Go to the finding |
| The project holds no map of its own | The Outline, under the project's name | Place or duplicate an object: the first one makes the map |
| No server is chosen | The stack line, *server: none* | Choose the folder… |
| A layer has nothing within its radius | The layer's row | Widen the radius |
| A kind of file is not checked in game | The Build tab, from the sheet of R70a | Open the guide's steps |

A tool that cannot act says why in a sentence at the pointer and writes nothing: an interior cannot be scaled, a locked file is kept whole, and a Legacy build converts an Enhanced model or texture where it does not convert a map taken from the other title's install.

## The first five minutes (R70c, R70e, R70h, R70i, R70q)

Stage 2 is the first thing handed to a mapper, and this flow uses only its rows.

1. The person opens the World room and flies. Ctrl+P takes a pasted position, a model name or a hash, and the camera goes there.
2. A click selects, and Details says what the object is and which map places it.
3. A move on a game object with no project open shows one line, *This object belongs to the game. Edit it in a project*, with the button. It opens New project… on the details step of the Map pack template, with the name and the folder as that step has them today, and asks where the map is for: a FiveM server or story mode.
4. The Map tab opens at the same camera with the same object selected. The object moves, and [the first edit](#the-first-edit-of-a-game-object-r70h) says what it brought into the project.
5. A prop is added by duplicating an object and swapping its model, since the Library comes with R70j ([placing](#placing-r70j)).
6. Delete removes an object of the project, and is refused on an object the game placed until R70k lands ([deleting](#deleting-r70i-r70k)).
7. Ctrl+B builds, and the Build tab says what was written and what was not checked in game.

## The camera, the pointer and the keys

The world has one camera with two states. With flying off, the state for editing, a letter is a command and never a movement. With flying on, a drag turns and the keys move. A World tab opens flying, a Map tab opens with flying off, and the status bar says which.

This replaces two pointer models that exist today. The GTA VI viewer today turns on a drag of any button and flies on its keys with no button held (`src/ui/panels/world_viewer.rs`). The Map scene today orbits on a drag, pans with the middle button or Shift and zooms with the wheel, and a double click there opens the model of a scanned GTA VI entity (`src/ui/panels/maps/scene.rs`). The model viewport today keeps the right button for its menu (`src/ui/panels/viewport.rs`). The choice is the ruling on the keys and the camera, whose default the rows take.

### The pointer (R70c, R70i)

| Input | Flying off | Flying on |
|---|---|---|
| Click | Picks; a second click at the same place picks what is behind | The same |
| Drag of a handle | Moves, turns or scales the selection | The same |
| Drag on empty ground | Draws a rectangle that selects | Turns the camera |
| Alt and a drag | Orbits the selection | Turns the camera |
| Middle button, or Shift and a drag on empty ground | Pans | Pans |
| Right button held | Flies while it is held | Turns the camera |
| Right click, released where it was pressed | The menu of what is under the pointer | The same |
| Wheel, or two fingers on a trackpad | The distance to what is under the pointer | The speed of flight |
| Double click | Opens the object's model; swings a door | The same |

A person on a trackpad never holds a button to fly: Shift+F turns flying on, one finger turns, two fingers set the speed, and Shift with a drag pans. A person with the keyboard alone turns flying on, moves with the fly keys and turns with the arrows; they select from the Outline, go anywhere from the box, and set a pose in the fields of Details.

### The keys (R70c, R70e, R70i)

Each key is read by the world's panel, under the pointer or with the focus, as the `panel_key` rows of the command table are today. None is taken from a command that has it.

| Key | Does | The binding today | Row |
|---|---|---|---|
| 1 to 5 | Select, move, turn, scale, collision | No command binds a bare digit; Ctrl+1 to Ctrl+9 go to a tab (`src/ui/commands/project.rs`) | R70i, R70l |
| Shift+F | Turns flying on and off | Unbound; F and the period are Frame selection (`src/ui/commands/table.rs`) | R70c |
| W A S D or Z Q S D, E or Space, C, Shift | Fly, and fly faster, while flying only | The viewer's keys; Space is also Quick look (`src/ui/commands/project.rs`) | R70c |
| Arrows, Page Up, Page Down | Flying on, the arrows turn the camera. Flying off, the keys move the selection a grid step, ten with Shift | The viewer flies on the arrows as on W A S D, which is given up; the Map scene moves the entity | R70c, R70i |
| Brackets, or R and Shift+R | Turn the selection or the ghost by the turn step | The Map scene reads the brackets, which an AZERTY keyboard types only with AltGr; R is unbound | R70i |
| End | Stand on the ground | The command has no key (`drop_to_ground` in `src/ui/commands/maps.rs`) | R70i |
| Delete, Ctrl+C, Ctrl+X, Ctrl+V | Remove, copy, cut and paste objects | The four commands, which have no target for a placed thing (`src/ui/commands/target.rs`) | R70i, R70k |
| Ctrl+D | Duplicates; pressed again, repeats the last offset | Unbound | R70i |
| F3, Shift+F3 | The next and the previous placement of the model gone to | Unbound; the text editor reads F3 itself | R70e |
| Alt+1 to Alt+9, with Shift to set | Go to a bookmark, or set it | Unbound; the key that sets needs the matcher's change | R70e, R70i |

Stand on the ground, Remove entity and Add an entity are the commands the Map screen has today, given keys and a target. Undo, Go to file…, the command palette, Build, Frame selection and Frame all keep the keys they have today.

### What the matcher has to change (R70i, R70v)

- **A binding by where the key sits (R70i).** The matcher today reads a press by the character it types and falls back to the key's position only when that character has no name (`key_of` in `src/ui/shortcuts.rs`). ⚠ On an AZERTY keyboard the fourth key of the digit row types an apostrophe, which reads as the Quote key and never as 4, so a tool bound to the character would be dead there without Shift. R70i adds a binding that names the position, and the five tools use it.
- **Shift with a digit (R70i).** The matcher today ignores Shift for a digit (`shift_picks_the_character`), so Shift with a digit cannot set what the digit goes to. A binding by position honours Shift. Until then a bookmark is saved from the box, and Alt with a digit goes to it.
- **A place for a binding (R70v).** A conflict is today one key for two commands of the whole table (`set_binding`). Every key above is unbound elsewhere, so nothing collides before R70v, which adds the place a binding belongs to; the preset for a person who knows CodeWalker's keys needs it.
- **Keys read outside the table (R70i).** The Map scene reads F, Escape, the arrows and the brackets itself today. R70i makes them rows of the table, so the page of keyboard shortcuts lists them.

A field being typed in keeps its keys today (`the_focused_field_keeps`), so a digit typed into Details never changes the tool.

### Escape (R70i)

Escape closes what is in front first, a popup, the box, the delete preview or the clean frame, and does nothing more in that press. Otherwise it cancels a drag and puts the object back, then ends a placement, then clears the selection, in that order. The box closes on Escape today without taking the key (`src/ui/panels/command_palette.rs`), so the view reads Escape only while it holds the focus, as the Map scene does today, and closing the box leaves a selection as it was.

### The status bar's hints (R70i)

The status bar has a hint today, shown while nothing is open, which says what a drop does on the screen in front (`src/ui/panels/status_bar.rs`). On a world tab that place says what the pointer and the keys do for the tool in hand, and it changes while a handle is dragged, a ghost follows the pointer or the camera flies.

## Working on objects

### Selecting (R70i)

1. A click picks what is under the pixel, and a second click at the same place picks what is behind it. A decal, a painted shadow and a pane of glass give way to the surface under them.
2. Ctrl with a click adds or removes. A drag on empty ground draws a rectangle that takes what is visible in it, and the Outline selects by name.
3. While a layer is shown its things are picked before objects, so a road node on a road is the node.
4. Details offers a selection of several kinds only what all of them take.

### A right click and a double click (R70i, R70n, R70s)

A right click released where it was pressed opens the menu of what is under the pointer, drawn from the command registry as `every_context_menu_draws_from_the_registry` holds the other menus today. On an object it offers Copy for script, Duplicate, Swap model…, Put back as the game has it, Delete and Open the model; on empty ground, Paste here and Place again.

A double click on an object opens its model in a tab. On a door it swings the door, once R70n lands, and Open the model stays in the menu.

### Moving, snapping and repeating (R70i)

1. A drag of the manipulator writes one step of Undo when it lets go, as today, and every handle has a typed field in Details and a key.
2. The snaps are checkboxes beside their numbers, as today: the grid, turns, scales, On the ground, and one more, Objects.
3. **Objects** snaps a moving object to the box of the one under the pointer: its faces, its corners and its pivot. A box is not the model's surface, so the snap can leave a gap.
4. Ctrl reverses whichever snaps are on, as it reverses the grid today: with the grid on and Objects off, a drag with Ctrl leaves the grid and takes the box under the pointer.
5. End stands the selection on the ground, which is the collision under it (R70g), where today it is the highest box.
6. A game object that moves takes the far models that stand for it alone, each file named before the step. A far model that also stands for a neighbour stays and is named.
7. **A line of props** is Ctrl+D, a move, and Ctrl+D again: the second press repeats the offset, so a row of barriers is one press an object, at equal spacing.

Taking another object's heading or height has no command: Details holds the numbers.

### Placing (R70j)

Before R70j lands a prop is placed by duplicating an object and swapping its model, which R70i builds.

1. Select any object and press Ctrl+D. A copy of a game object is a new object of the project and imports nothing.
2. Type the model's name into the archetype field of Details, which the Map screen has today, or choose Swap model….
3. Move it, and press End.

The first new object of a project with no map makes the map that receives, from a donor of the mounted install (R70h).

Once R70j lands:

1. The Library tab lists the props of the project, of what it links and of the game by category, with a search.
2. A prop is dragged into the world, or chosen and then clicked into place, and a ghost follows the pointer on the surface under it.
3. While the ghost follows, a key acts on the ghost: R turns it, Page Up and Page Down raise and lower it.
4. Each click places another. Escape or a tool key ends the placement, and what was placed last is then selected, so the next key adjusts it.
5. Ctrl+D after two placements repeats their offset, and Place again brings the ghost back with the last prop.

Thumbnails wait for the ruling on what is built in the app and not in a plugin: until it is given a row is a name and a category. Reading the XML a Blender export writes, and the folder watched for it, wait for the ruling on that XML.

### Deleting (R70i, R70k)

- **An object of the project** is removed by Delete, as one step of Undo (R70i).
- **An object the game placed, before R70k lands,** is not removed. The refusal at the pointer says that its far models and its collision would stay, since the removal the Map screen has today works inside one file.
- **An object the game placed, once R70k lands,** opens a preview and not a question. It lists what goes, the object, its far models and the parents left with nothing, and what stays and why. In the world what goes is hatched and what stays is outlined, each listed by name with a box to untick for each file. One confirmation makes one step of Undo.
- **The collision and the occluders of the building** are proposed in the same preview by R70l and R70o, and until they land the preview says that both stay. The proposal leaves the ground under the building; an option, off to begin with, takes it too.
- The menu copies the script line that hides a model.

### Inside an interior (R70m)

The objects of an interior are entries of its archetype in a type file, in the interior's own frame, each attached to a room. No map holds them.

1. While the camera is inside, the end of the stack line names the interior and the room.
2. A prop placed inside joins that archetype and the room under the pointer, not the map that receives, and Details shows its position in the interior's frame.
3. The first edit of a game interior's furniture imports the type file that declares the interior, with its base, and the toast names that file.
4. Entity sets are switched from Details, each with the script line that turns it on.
5. Until R70m merges, a tool refuses an object of an interior with a sentence.

The edit of a room, a portal and an entity set, from Details and by handles, and an interior made from nothing wait for the ruling on what is built in the app and not in a plugin. A moved game interior says that its sound no longer matches, until R70x writes its audio.

### Undo and History (R70h)

Undo and Redo act on the tab on screen ([R69h](Roadmap.md#r69h-one-undo-for-the-tab-on-screen)). Today the Map screen's Undo is a stack of its own, held in memory and reset with the project (`src/state/workbench/maps.rs`). R70h makes each world step a step of the project's History, which is kept across sessions ([what project history builds](Workbench_UX_Design.md#what-project-history-builds)). History keeps today a bounded count of steps (`KEPT` in `crates/domain/or-project/src/snapshot.rs`), so the list of [what the project changed](#what-the-project-changed-and-putting-it-back-r70y) is the way back once a step has gone.

## The project and the game's map

### The first edit of a game object (R70h)

1. Moving or changing an object the game placed imports that object's map into the project as a source that keeps the game's file beside it. No question is asked.
2. A toast says what happened in one line and has Undo, which takes back the edit and the import as one step. It is not the Undo import an import's toast has today, which removes nothing once the source changed.
3. The toast also says that the map is a source taken from the game. Today the managed block of the project's Git ignore file leaves such a source out and Check before sharing lists it ([commit policy](Source_Format_Design.md#commit-policy)), so a clone opens without its changed maps until Commit sources taken from the game, a control of the project's Properties, is turned on. Whether a map project should behave otherwise is a ruling asked, whose default keeps the policy and has the toast and the guide say so.

The first edit is refused, with a sentence and no file written, in four cases: the map was taken from an install of the other edition than the build's target, and the sentence names an install of that edition as the way out; the map's XML does not come back to itself; the object belongs to an interior before R70m; the tab is a World tab.

### What the project changed, and putting it back (R70y)

1. The Changes tab lists each object the project added, moved or deleted against the game's file, by map, with a count for each kind and a filter. Two twins that cannot be told apart are listed as undecided and never as deleted.
2. A click on a row selects the object. A deleted one draws nothing, so its row moves the camera to where it stood and draws its outline while the row is selected.
3. **Put back as the game has it** restores one object from the kept base, as one step. It is on the row, in Details and in the object's menu.
4. A deleted building is one row for the object and the far models that went with it. Putting it back restores them together, with its collision and occluders once R70l and R70o have landed.
5. **Discard changes to this map** does it for the whole source, after a question that counts what it undoes.

Before R70y lands the way back is Undo and History.

### Coming back after a game update (R70b, R70r)

1. Opening the world builds the index again, as a job, when an archive of the install changed. The camera, the bookmarks and the stack are as they were, and a stack pinned to a pack stays there.
2. Each source whose base is no longer the install's file is then a Problem that names it and offers the merge that carries the project's changes onto the new file. The merge is one step, and what it cannot carry is listed object by object.
3. Today the check against the game runs only when a person asks it, with `openrage-cli project check-original` or from a source's row.

## Finding and reading

### Finding (R70e)

1. The box lists commands and files today (`src/ui/panels/command_palette.rs`). It gains places: a position in any form a script writes it, a hash in hexadecimal or as the signed number a script prints, a model, a map, a type.
2. On a world tab Ctrl+P opens the box. Today, with a file open, that key puts the focus in the field of Files (`go_to_file` in `src/ui/panels/hierarchy/navigation.rs`); R70e lists places there as well.
3. A query that reads as a position or a hash lists its place first, ahead of files. A position of two numbers takes its height from the ground under it.
4. **A model placed many times.** Its row says how many times it is placed. Enter goes to the placement nearest the camera and selects it, and the status bar says which one of how many. F3 and Shift+F3 step to the next and the previous, nearest first, and the row's Open button opens the model's file. On a model nothing places, Enter opens the model.
5. A map's row frames its extents. A bookmark is a place the person saves, found in the box by its name and gone to by its key.

### Layers (R70f)

A panel of switches with presets named after tasks, such as mapping, roads and traffic, interiors and occlusion. A layer draws within a radius of the camera that its row changes. The maps only a script requests are a list with a search, the section of script-managed groups the World panel has today for GTA VI. A layer appears when its records are read: zones, far lights and a parked vehicle's fields come with the data part of R70o.

### Details and Copy for script (R70e, R70i)

Details says the object in a mapper's words first, with the raw fields behind Show internals. Rotation is shown in degrees, a flag with a name is a checkbox, and Placed by lists the nearest placement first.

Copy for script is a button with the last form used and a list of the others: a position, with or without its heading, the model's name, its hash in both spellings, a list of the selection, and the lines that spawn the object.

### Problems (R70h and the rows that add rules)

The rules run on the project as it is edited. A finding says what is wrong in a sentence, goes to the place when clicked, and offers a fix where the fix is safe, as Fix all safe does for the other contents today. The Build control carries the count, as it does today, and each row names its rules in the roadmap.

### Browsing models and vehicles (R70z)

The browser lists the game's object types and vehicles with filters by class, by pack and by size, favourites and the models opened last. For the model shown it lists where it is placed, nearest first. It saves the view as a picture, and draws a vehicle in the colour combinations the game plans for it, with its sheet. Thumbnails wait for the same ruling as the Library's.

## Building and the server

### Building (R70q)

After a build for a FiveM server the Build tab shows the line to add to the server's configuration, with Copy. After a story build it states the line the DLC list needs, as it does today (`src/ui/panels/project/build_part.rs`), and Settings says today whether the game reads a mods folder (`src/ui/panels/settings_overlay_loader.rs`). Installing the pack into the game's mods folder waits for the owner: the build writes the pack and says where it goes.

### The server folder (R70r)

1. **Where it is set.** From the stack line at any time, which reads *server: none* until a folder is chosen; in the New project step, when the map is for a FiveM server; or at the first build to a server. It is remembered for this machine and never in the project file, as the folder a build is copied to is today.
2. **What it changes.** The folder is a read-only layer of the world: the server's maps draw over the install's. Reading the folder is a ruling asked, whose default is yes.
3. **When a conflict is named.** At the edit that imports a map another resource ships, as a Problem with Compare and Merge, and again before a build writes. The import takes the game's file as the base and offers the resource's copy as changes to merge.
4. A bound, a road file or an occlusion file that two resources ship is named and not merged.
5. The build warns when its folder lies where the server does not look, and names an earlier build of the same resource elsewhere.

## The later screens

- **Collision (R70l).** The fifth tool: polygons picked and removed, a material set, the versions of an area written together, into the source R70w builds.
- **Doors (R70n), and parked vehicles, zones, lights and occluders (R70o).** Each is picked on its layer and edited in Details, with its rules in Problems.
- **Roads, scenario points and the navmesh (R70p).** A node is moved, added or disabled; one another region names is never deleted.
- **The model workshop (R70s).** Before an edit of a game model it says how many placements use it, and Save as a new prop swaps only the object it was opened from. A prop made from an image waits for the ruling on a drawable made from nothing.
- **Peds, cutscenes and life (R70t).** Walking the map waits for the ruling on what is built in the app.
- **Working together (R70u).** What a person holds is locked to the others, a guest whose stack differs is refused with what differs, and only the host builds.

## Words, languages, colour and reach (R70v)

- **Words.** Screens say Legacy and Enhanced, and call a `.ymap` a map and a `.ytyp` a type file, as the Map screen does today.
- **Languages.** Every sentence of the screen goes through the catalogs of every interface language ([Interface_Languages.md](../guides/Interface_Languages.md)). The sentences of the map rules do not today: a library crate's sentences stay English, so a finding of Problems reads in English on a French screen until R70v wraps them.
- **Colour.** Colour is never the only sign of a layer, a level of detail, a category, a change or a preview: each also has a pattern, a glyph or a row of a legend. That is a rule for the rows and not a property of the tree: today the Scene tells a level of detail by colour alone.
- **Reach.** Every new control has a name a screen reader speaks and a row of the [control inventory](Workbench_UX_Design.md#control-inventory), which `every_control_of_the_project_screens_has_a_row_with_its_place_and_its_reason` holds today for the screens of a project.
- **Settings and the guide.** R70v gathers on one page what a mapper changes, the camera's sensitivity, the movement letters of the keyboard's layout and the snap steps among it, and adds the mapper's chapter of the guide.

## What waits for the owner

- **A default is taken** until the owner says otherwise: the keys and the camera, with the presets; the first edit that imports with no question; a server's resources folder read as a layer; a changed game map left out of a shared repository; a live session that reaches the network when a person starts one.
- **It waits** for the answer: what is built in the app and not in a plugin (thumbnails, the edit of rooms and portals and an interior made from nothing, walking the map); reading the XML a Blender export writes; a drawable made from nothing (a prop made from an image); installing a story pack into the mods folder.

## Sources

- [Workbench_UX_Design.md](Workbench_UX_Design.md) and [GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md#the-camera), for the shell and the camera as built.
