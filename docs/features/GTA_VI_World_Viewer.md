# GTA VI World Viewer

The World screen flies a free camera through GTA VI's world as the game streams it at rest. It
reads the install the GTA VI folder of Settings ▸ Games names, and writes nothing into it.

## Opening it

The rail shows **World**, a globe with a small gamepad, once a GTA VI folder is set. It opens the
World workspace: the view, and beside it the World panel, which says what the mount found, what is
streaming and where the camera is.

The first visit builds an index of the install as a job with progress and Stop. The index is kept
in the configuration folder under `gta6_world`, and is built again only when an archive's size or
modification time changes, or from **Build the index again**.

## Which maps it loads

A GTA VI archive's table is ciphertext, so no file is opened by its path. The viewer mounts the
install by what its bodies state (`crates/title/or-title/src/gta6_mount.rs`):

1. It scans the small archives for the registration documents in the clear: each pack's setup, its
   content definitions and their change sets. A change set a document includes is found by its id.
2. The level pack's level sets list the archives the level registers. A registered archive's name
   is the name of a map inside it, and every map names such a map as its block, so each map body
   is bound to a registered archive or left out.
3. A map of the level streams at rest when it is not script managed and holds something to draw.
   A map a script switches on is not part of the world at rest and is not loaded.

An archetype is found by its 64-bit word among every map types body of the level's archives, and a
model by the word its drawable or fragment states, or by the one drawable dictionary that holds a
drawable under every name of a dictionary's archetypes.

## Streaming and level of detail

`crates/domain/or-world/src/streaming.rs` holds the rules, with no file or device in them:

- A map is wanted while the camera is within 50 m of the box it streams by. Its level-of-detail
  parent maps are wanted with it. A map's body is read when it is wanted, on a worker.
- A node with a parent is judged by the distance to its **parent**, so the children of one parent
  appear together. A parent gives way to its children once the camera is within the distance it
  states for them, every child map is loaded and every child's model is on the device; until then
  the parent draws, so no hole opens while models arrive.
- A list of instances draws by the distance to its box, less each instance whose own entity draws.
- The **Level-of-detail scale** multiplies every distance.

Streaming and selection run on a thread of the world's own, which selects again when the camera
has moved, a map came or went, or a model did. A frame reads the last selection and never waits
for one.

## Drawing

A selection's instances stand in chunks, a map's entities or one list each, with the sphere that
holds them. A frame culls the chunks and then the instances to what the camera sees, on every
core, and lays the instances it sees side by side a model; when neither the camera, the selection
nor what is on the device changed, it does nothing. Each model is one vertex buffer and one index
buffer a mesh, drawn once for all its instances in view
(`crates/runtime/or-renderer/src/world.rs`). In Shaded the instances are written into the
frame the game's own lighting passes light, where a game shader pack is read; with none they are
lit by the plain pass. The World panel says which.

Models are read on workers, the nearest first, and textures as the models that draw ask for
them; both are put on the device under a budget a frame; the ones drawn longest ago go once the device holds more than its budget. A texture is read
at a small level of its chain, never its largest.

## The camera

| Input | Does |
|---|---|
| Drag in the view | turns the camera where it stands |
| W A S D, or Z Q S D | forward, left, back, right |
| E or Space, C | up, down |
| Shift | four times as fast |
| Mouse wheel | multiplies the speed, from 1 to 2,000 m/s |

The keys are read while the pointer is over the view or the view holds the focus.

## Limits

- Grass lists are not drawn, and an interior's contents are not placed.
- A time archetype draws at every hour.
- A level of detail swaps at its distance; the game fades across 20 m.
- The sun casts no shadow over the world.
- Every model draws at its first level of detail.
- The packs of a game mode are not mounted: no document in the clear states when their maps load.
