# GTA VI World Viewer

The World screen flies a free camera through GTA VI's world as the game streams it, at an hour,
in a weather and in the game's modes the World panel sets. It reads the install the GTA VI folder of
Settings ▸ Games names, and writes nothing into it.

## Opening it

The rail shows **World**, a globe with a small gamepad, once a GTA VI folder is set. It opens the
World workspace: the view the whole height of the window, and beside it the World panel, with
**Jobs** and **Log** as tabs behind it. With no folder set, both say so and offer
**Settings ▸ Games…**. While the World screen shows a GTA VI folder, the status bar names GTA VI,
and says **No game** once that folder is found to hold no world. A World arrangement saved before
the view took the whole height starts over once from this one; the other workspaces keep theirs.

The first visit builds an index of the install as a job, **GTA VI world index**, with progress and
Stop; until the index is held the view is a plain backdrop. The index is kept in the configuration
folder under `gta6_world`, and is built again only when an archive's size or modification time
changes, or from **Build the index again**. A folder that holds no GTA VI files fails the job, and
the view, the panel and the job list say that no GTA VI files were found in it and that the folder
to name is the one GTA VI is installed in, which holds its `.rpf` archives.

The World panel lists its sections in the order they are used:

1. What stands in the way, each with what to do about it: the index being built, with its
   progress; a build that failed, with **Settings ▸ Games…** and **Build the index again**; no key
   snapshot (the maps are then bound by their block maps, see below); and a frame lit by the plain
   pass because no shader pack is read.
2. **Time and weather**: the hour and the weather the world is drawn at.
3. **Camera**: **Go to**, the overhead map, where the camera stands in words, the interior and room
   it stands in, its position, its speed and the level-of-detail scale.
4. **What streams**: the story pack, the shared minigames and the script-managed groups.
5. **Capture**, closed until opened.
6. **Streaming**, what is streamed in and on the device, and **Mount**, what the index holds, closed
   until opened.

The line over the view says how to fly. The corner under it says what is still loading (maps,
models, textures, maps' lights, the ground and fog under the water) and goes away once nothing is.

The hour, the weather, the modes, the level-of-detail scale and the camera's speed are kept in the
preferences and come back on the next run; the groups chosen and the camera's place do not.

## Which maps it loads

The viewer mounts the install by what its archives state (`crates/title/or-title/src/gta6_mount.rs`).
How it finds a file depends on whether a key snapshot is at hand: `gtavi_magic.dat` in the
configuration folder, or the file `OPENRAGE_GTA6_KEYS` names.

- **With a snapshot**, each top-level archive's table is deciphered, and the registration documents
  (each pack's setup, its content definitions and their change sets) and the archives they register
  are found by name. Every map body is then bound to the registered archive that holds it.
- **Without one**, the small archives are scanned for the documents in the clear, and a map is bound
  through its block, a map whose name is the registered archive's.

The World panel's **Mount** says which of the two bound the maps, and with no snapshot the top of
the panel says where to put one. A change set whose file is ciphered is not read.

Every pack states its mode: the base game, the story pack or the shared minigames. A map of the base
game streams when it is not script managed and holds something to draw. **What streams** in the
World panel switches the story pack's and the shared minigames' maps on.

A script-managed map belongs to a group of maps a script switches together. With a key snapshot, the
world-state selectors of each mode's game-shape archive pick, hour by hour, which script-managed
groups are on, as a new game shows them; without one, none is. A selector with no rule that holds
takes its default. Only the selectors of the layers the level loads count, and of the layers a
configuration they pick loads in turn: a layer only a mission, a script or a system asks for switches
nothing at rest. A rule on the player reads as a new game: no wanted level, nothing unlocked, nothing
held, no time yet spent in a configuration. Of two requests on one selector the higher priority
wins, the first made of equal ones. What a fresh game is taken to have beyond the data, the requests
its scripts make and the flags it starts with, is read from `fresh_game.json` beside the kept index
in the configuration folder when one is there; a group it switches is marked inferred in the list,
and **Only what the data proves** leaves it out. The same section lists the groups,
and a group ticked there streams its maps in; from then on the hour no longer sets them, until
**The game's own groups** puts them back.

An archetype is found by its 64-bit word among every map types body of the indexed archives, and a
model by the word its drawable or fragment states, or by the one drawable dictionary that holds a
drawable under every name of a dictionary's archetypes. A time archetype draws only inside the
hours it states.

## Streaming and level of detail

`crates/domain/or-world/src/streaming.rs` holds the rules, with no file or device in them:

- A map is wanted while the camera is within the sphere of the box it streams by: about the box's
  middle, of the box's smallest half extent and 50 m; a box under a metre of half extent on any
  side streams in nowhere. Its level-of-detail parent maps are wanted with it. A map's body is read
  when it is wanted, on a worker.
- Every tier of map streams in at any speed, and nothing is asked for ahead of a moving camera.
- Every node draws by its own distance. An entity that stands in for the whole of its parent is
  judged by the distance to its **parent**, a list by its own box's centre, and the children of one
  parent draw as opaque as the most opaque of them.
- A parent fades out as its children fade in, taking what the most opaque of them leaves, once as
  many as it states are loaded (an entity's count rises to those loaded) and while its own distance
  leaves it whole. It is
  held whole while a child that draws has no model on the device yet, so no hole opens while models
  arrive; such a child is asked for even where its own children draw in its place. A parent fading
  out for its children keeps the pixels their dither leaves, so a cross-fade leaves none bare. A
  model something in view waits for counts as on the device only once the textures of its levels
  are there too, so no level draws grey while they arrive; its textures are asked for as its read
  ends, before a frame has room to put it on the device. A selection lists what it waits for as
  it will draw, so the frame a model is ready in draws its node and the parent it held as the next
  selection will, without waiting for that selection. A prop list of the batch levels never fades out for its children: each list under it of
  the batch LOD level leaves out the instances it stands in for, all of them where it names none.
- A node is whole within the distance it draws to and fades out over a band past it: 20 m for the
  high, orphan, first and second levels of detail, 25 m for the three farthest, 5 m for an orphan that
  draws to 20 m or less.
- A far node with no parent that arrives in view fades in over five selections. A node of the high
  level, an orphan and any node with a parent are there at once, and so is everything after the camera
  jumps more than 5 m, moves faster than 80 m/s or the scale changes.
- A distance is scaled by what the archetype is: terrain and environment draw shorter at the near
  levels, a grass list at 0.8, keeping the part of the distance inside the model's own radius whole;
  under a parent that is an entity the parent's radius is kept instead, so the children of one
  parent that state one distance draw to one distance and leave no hole where it gives way.
  A prop list is never drawn shorter than it states.
- A script-managed map that reads with nothing to draw is let go and not asked for again; a map
  wanted as the parent of another is kept whatever it holds.
- A list of instances draws by the distance to its box's centre, less each instance whose own entity
  draws; that entity fades out over its band by its own place.
  A grass list draws each of its instances within the distance the list states for one instance,
  scaled with the list, fading out towards it.
- An interior a map places draws its contents, and the entity sets its instance names, while the
  camera is near it.
- The **Level-of-detail scale** multiplies every distance, and a lens narrower than 45° draws farther.

The maps of occluders stream by a tier of their own, on workers of their own, beside the maps the
forest draws.

Streaming and selection run on a thread of the world's own, which selects again when the camera
has moved, a map came or went, a model did, or a fade is still under way. A frame reads the last
selection and never waits for one. What a selection waits for is asked for as soon as the forest
is walked, before its instances are laid out. Models and textures get as many readers each, half the
machine's threads up to twelve. What the readers answered goes on the device soonest needed first,
textures before models, for 4 ms a frame or a quarter of the last frame where that is longer, and
they read no more than 512 MiB ahead of what frames take.

## Drawing

A selection's instances stand in chunks, a map's entities or one list each, with the sphere that
holds them. A frame culls the chunks and then the instances to what the camera sees, on every
core, gives each instance the level of detail its distance draws at, and lays the instances out a
model and a level; when neither the camera, the selection nor what is on the device changed, it
does nothing.

- The occluders of the occluder maps around the camera, the largest on screen first, are drawn
  into a small depth image of their own, and a chunk whose sphere they hide is not laid out.
- A model keeps every level of detail its drawable holds. Its meshes are written into shared pages
  of vertices and indices, and draws of one page and one material go out together where the device
  can draw several at once (`crates/runtime/or-renderer/src/world.rs`).
- A mesh is drawn with its whole material: its family (one, two or four layers, decal, glass, grass,
  terrain LOD, emissive), each layer's colour, normal and material maps, its lookup, tint and palette
  maps and its constants, in the bucket its shader states: opaque, cut out, emissive, decal and
  blended surfaces draw in that order. A surface that only marks water for the game is not drawn.
  Metalness and roughness come from the material maps, and the two vertex colours carry the palette
  column, the layer weights, the tint and the glow.
- An emissive surface glows by its material's strength, its map and its second vertex colour, going
  to its night value by the day's own blend, at the day's exposure.
- The far stand-ins of vegetation draw as cards: one quad a stand-in, facing the camera, in the
  passes and buckets the cut-out meshes draw in and into the sun's shadow. Each species is baked
  once, from a fixed set of directions, into an atlas from the least detailed level of its own
  model, once that model and its colour maps are on the device; a species the world never draws
  near is read for its atlas alone, after everything that draws, and until it is baked its cards
  draw nothing. Which species each stand-in names is worked out at index build from the far
  stand-in lists of every map. A list of stand-ins fades out as the lists under it fade in, by the
  rules of any parent.
- A cut-out surface is cut by its colour map's first level, whatever level the screen samples, so
  that leaves and grass keep their coverage far from the camera; a far stand-in's card the same
  by its atlas.
- A grass instance is tinted by its own colour, as far as its material's grass tint and the
  vertex's second colour let it. A prop instance's own tint is the whole colour of a material of
  the region-tint effects of the far prop lists, under the vertex's first colour as far as its
  alpha in their per-vertex variant, and no part of any other material.
- A fading instance is drawn through an ordered dither, the same in every pass.
- In Shaded the instances are written into the frame the game's own lighting passes light, where a
  game shader pack is read, and the sun casts their shadow in four cascades about what the camera
  sees; with no pack they are lit by the plain pass. The World panel says which.

Models are read on workers in the game's order: what waits for them in view first, then far
levels, interiors and the rest, and the nearest first within each; textures as the models that draw ask for
them; both are put on the device under a budget a frame, and the ones drawn longest ago go once the
device holds more than its budget. A texture is read at the level the screen needs and every level
under it, from the rows of its pack those levels lie in; block-compressed rows go to the device as
they are stored. A texture none of whose rows decode is a mid grey.

## The day

The World screen is lit by GTA VI's own day, read from the install's timecycle, weather and visual
settings: the sun's and the moon's place at the panel's hour, and the weather the Weather bar names,
its cycle with its modifiers laid over it. The sun's colour, the sky's dome and the fog's colours
come from a model of the atmosphere with the install's own values; the frame is exposed by the
install's exposure curve, metered on the light of the moment, and the tone curves are the visual
settings'.

The lights around the camera are those the archetypes of its entities hold, those the entities
place themselves and the maps' level-of-detail lights where no placed light stands, each lit in the
hours it states and scaled by the day's exposure; the brightest are the frame's point and spot
lights. A level-of-detail light fades out at the reach its size class has in the visual settings.

The level's water is drawn as a blended surface over its quads, coloured by its depth over the
ground the level's height map gives and by the day's water.

## The camera

| Input | Does |
|---|---|
| Drag in the view | turns the camera where it stands |
| W A S D, or Z Q S D | forward, left, back, right |
| E or Space, C | up, down |
| Shift | four times as fast |
| Mouse wheel | multiplies the speed, from 1 to 2,000 m/s |

The keys are read while the pointer is over the view or the view holds the focus.

**Go to** lists up to eight places found in the index when it is opened: the squares of 512 m that
hold the most maps streaming at rest, densest first, one for each name. A square is named by the
place the world-state data outlines at its middle (a region such as Little Cuba, an island or a
district), else by the run of letters most of its maps' names start with (the letters before the
first digit or underscore), else by its coordinates. Choosing one puts the camera 250 m south of the middle of its maps and 80 m over the
tallest of them, looking north.

The **overhead map** under it is drawn when the index is opened: the square over the maps that
stream at rest, north up, each cell as bright as the number of maps whose middle lies in it, over
the land and the water. The land is shaded by the height the level's height map gives it, lit from
the north-west; a cell is water where a quad of the level's water stands over its ground, or, past
every quad, where its ground lies under the sea. Both are drawn once the viewer has read them. It
takes at most three tenths of the panel's height. The camera is a dot with a stroke the way it
looks. A click puts the camera 250 m south of the point
clicked and 80 m over the tallest building within 256 m of it (or the ground, where none stands),
looking north.

The lines under the map say where the camera is in words: **In** the smallest place the world-state
data outlines around it that the vocabulary names (by its label, said as words: the label of Vice
Beach is region, vice, beach, name), then the smallest map at rest under it whose
name is known, how high it is over the ground the level's height map gives, and which of the eight
points of the compass it looks to.

Names come from the vocabulary the app serves: shared bundled word candidates, the names
**Harvest names** found, and verified names in the local `gta6_names/resolved-names.tsv` cache.
The folder `OPENRAGE_GTA6_NAMES` names can supplement these with `vocabulary.txt` and name
tables; external names take priority over local names, which take priority over bundled candidates.
This vocabulary is read once on a worker. Bundled files contain plain words shared across titles;
complete recovered paths and game file extensions stay in the user's files. A place, a map, an
interior, a room or a script-managed group the vocabulary does not name is shown by its hash.

## Time, weather and what streams

- **Hour** sets the hour of the day: what is placed for some hours draws inside them, and the world
  is lit by its day at that hour. It stops a running day, so the Time bar shows the same hour.
- **Weather** sets the weather the day is drawn in; it is the same weather the model viewport's
  Time bar sets.
- **Story pack** and **Shared minigames** stream the maps those packs add.
- **Script-managed groups** lists every group of the index with its maps; a group ticked streams in.
  The line under it says whether the groups on are the game's own at this hour or the ones chosen,
  and the field above the rows keeps those whose name holds the text typed.

## Capture

The **Capture** section of the World panel makes a frame that can be drawn again.

- **Clean frame** draws the world alone over the whole window, with no panel, line or toast over
  it. Escape brings the panels back.
- **Copy the camera** copies where the camera stands and looks as one line,
  `world camera X Y Z YAW PITCH FOV`: metres, then the yaw, the pitch and the vertical field of
  view in degrees. A yaw of zero looks along +Y and a growing yaw turns towards −X.
- The field under it takes such a line and puts the camera there on Enter.

Each of these jumps, like **Go to** and a click on the overhead map, is a cut: the exposure is taken
again at the new place rather than adapted from the old one.

A shots script reads the same line, sets the hour, waits for the world to settle (in the plain pass,
which meters no frame, without waiting for the exposure; and until the world's thread has selected on
every model it was told of) and measures
the frame against a reference image ([screenshots](../guides/Development_Workflow.md#screenshots)).

## Limits

- A far stand-in's card is baked from its species' own model, not read from the game's own atlas,
  and its colour and normal are the bake's, not the game's.
- A cut-out surface cut by its first level sparkles as the camera moves.
- A tree's own imposter model, which the game draws as a card, is drawn as a plain mesh.
- An interior's contents come and go at their distance with no fade, and draw at every hour.
- The atmosphere scatters light once, so the sky and its light are dimmer than the game's; the
  clouds are not drawn. The exposure is metered on the moment's light, not on the frame drawn.
- A light falls off by the frame's own curve; capsule and area lights draw as points, a spot's inner
  cone is the frame's, and coronas and light volumes are not drawn. A distant light with no partner
  is not drawn.
- The water has no waves, reflection or refraction, and its colour by place (the level's fog atlas)
  is not read: one medium colours all of it by depth.
- The occluders hide whole chunks, never an entity alone.
- A blended surface casts and receives no shadow.
- A change set's conditions are not evaluated: every change set read counts.
