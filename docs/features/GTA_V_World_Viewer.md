# GTA V World Viewer

The World screen flies a free camera through the world of a GTA V install, Legacy or Enhanced, as
its maps stream in, drawn by the renderer's plain pass. It reads the install and writes nothing
into it. What it does not draw yet is listed at the end.

## Opening it

The World screen shows the world of the **active install**. Make a GTA V install the active one
(Settings ▸ Games, or the game switcher of the status bar), wait for its keys to be read, and
choose **World** on the rail. With no active game, or one no world is opened for, the screen shows
the GTA VI folder's world as it did ([GTA_VI_World_Viewer.md](GTA_VI_World_Viewer.md)). Making
another install the active one ends the world held and opens the other.

The first visit reads every archive of the install into an index, which takes several minutes;
the view says how far it is. The index is kept in the configuration folder under `worlds/gta5`,
one file a world, so a later visit opens at once and reads again only an archive whose size or
time changed. A folder that is no install says why it did not open, with **Settings ▸ Games…**.

The view is flown as GTA VI's is: drag to look, W A S D or Z Q S D to move, E and C up and down,
Shift faster, and the wheel sets the speed. The corner says what is still loading.

## The panel

The panel beside the view shows the sections the opened world states, by its capabilities and its
title's options, and none of GTA VI's own:

| Section | What it does | Shown when |
|---|---|---|
| What stands in the way | the index being read with its progress, or why the world did not open | the world is not open |
| Hour | the hour of the day: a map placed for some hours streams inside them, and the sun stands where the hour puts it | the world is open |
| Go to | puts the camera at `X Y Z` in metres, keeping where it looks, or at a pasted camera line; the camera's own line is shown below to copy | the world is open |
| Options | the title's choices at open: for GTA V the DLC pack the mount stops at, by the name `dlclist.xml` gives it, and the map as the story or as multiplayer has it. **Open the world with these** opens it again | the title states an option |
| Streaming | instances in view and draws, models and textures on the device with their megabytes, and what is waited for | the world is open |
| Weather | that no weather is named, so a map that streams under some weathers alone stays out | the world states a weather gate |
| Could not be read | each reason a map, a model or a texture gave nothing, with how many | something was not read |

A world opened with other options is another world: it has an index of its own, read from the
install the first time.

## How it is read and drawn

Nothing is read on the thread that draws. The session opens the install through the registry of
titles (`or_worlds`), streams and selects with the world core's `Driver` on a thread of its own,
and reads maps, models and textures in the core's reader pools. A frame takes what arrived, puts
it on the device, and lays out the instances of the last selection the camera sees.

- **Models** go to the device from the game's own vertex bytes by their layout: position, normal
  and the first texture coordinate. A mesh that states no normal takes the facing of its
  triangles.
- **Textures** are asked smaller the farther what they draw stands, and read again larger as the
  camera nears. ⚠ No texture of GTA V states its colour space, so a map bound as a surface's
  colour is read as sRGB, as the game's programs read a diffuse map.
- **A surface** is one colour map and a bucket: opaque, cut out where the game's draw bucket says,
  or blended. The frame is lit by one sun, whose way the hour sets.

## Checking it without a window

`just shots` draws it: `game legacy FOLDER` or `game enhanced FOLDER` makes the install the active
one, `workspace World` shows the screen, and the `world` steps `camera`, `hour`, `clean`, `settle`
and `dump` act on it ([screenshots](../guides/Development_Workflow.md#screenshots)). `world
converge` is a settle here, and the other `world` steps are GTA VI's alone and refuse. A dump is
JSON: what is waited for, what was drawn, the bytes on the device, what a frame took, and every
reason something was not read with its count.

## What it does not do yet

- **The game's own look.** No shader of the game runs: no shadow, sky, cloud, fog, water (the sea
  and the lakes show the clear colour), emissive glow, tint palette or normal map. The day is the
  plain one of `WorldLook::frame`, not the install's time cycle.
- **Lights and occluders.** A map's far lights and occluders are read and ride with its body, and
  nothing lights or culls by them.
- **Instance lists.** Grass and prop batches are counted by the adapter and not laid out, so
  fields and verges are bare.
- **Interiors as rooms.** An interior's entities stream in within its distance and draw whole; no
  room or portal hides any.
- **Weather.** No weather is named, so a map gated by a weather stays out.
- **Textures of two formats.** A DXT3 or an A8 texture is not read, and its surface is drawn grey;
  the panel counts them.
- **Picking, selecting and editing.** The screen is read only.
- **A memory budget of its own.** Models and textures are let go by the budgets the GTA VI world
  uses, the longest unused first.
