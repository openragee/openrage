# File support

What the editor and `openrage-cli` do with each kind of file beyond the read and write columns of [the README's matrices](../../README.md#games), and where each stops.

## GTA V files in the editor

- `.ybd`, `.yvr`, `.ywr`, `.ynd` and `.ynv` open and save, the last two as kinds of their own with an outline. A `.ynv`'s polygons and leaf sectors are drawn in the viewport, and the outline's menu disables a polygon or deletes one. A delete is refused with the edges, special links and cells that still reach the polygon, and whether the game skips a disabled polygon is untested. `openrage-cli navmesh disable` and `delete` make the same edits on a file.
- A `.ysc` opens as decompiled source in the Data workspace, with its disassembly listing in the inspector, and `openrage-cli script` reads one with no window.
- An `.awc` lists its streams, plays them, exports them to WAV and replaces a stream from a WAV in the codec it has, PCM16, ADPCM or MP3, with a multi-channel stream's channels in PCM16 or ADPCM. Enhanced's MP3 streams and Legacy's XMA streams decode, replacing an XMA stream is refused by name, and no replaced stream has been played by the game. `openrage-cli awc` works on one too.
- `heightmap*.dat`, `waterheight.dat`, `worldblanket.dat` and `navprecalc.dat` open as images of their grids in the Texture workspace, a loose `.dds` there as a texture whose mips are viewed, exported and replaced, and `distantlights*.dat` as a list of light groups.
- The `*_cache_y.dat` cache files, `metadata.dat` and `audioworldsectorsinfo.dat` open as tables of their records.
- A `.rel` or `.datNN` lists its items by type with their fields, read-only.
- A `.gfx` lists its tags, texture references and imports, exports to SWF and is replaced from one, as `openrage-cli gfx` does.
- A `.mrf` shows its state machines, states and transitions with their conditions, and its nodes by kind.
- A `.cut`, a binary `.srl` and metadata XML no schema declares, such as `images.meta` and `weather.xml`, open in the metadata editor.
- A `.nametable` opens as its names, one per line, in the Data workspace, and a text `.dat` table or `preload.list` as text.
- The level's `water.xml`, and a `water_*.xml` a pack adds, has no view of its own in the editor. `openrage-cli convert info` reports its water, calming and wave quads and whatever else it states, and `convert` and `convert verify` take it by its name; no install's file has been read yet.
- Any other file opens as text when it is text, and otherwise read-only as its bytes.

## Gen8 and Gen9

Both editions of GTA V are supported natively, and a file's layout comes from its own
RSC7 version, never from the install it came from: an Enhanced install ships
Gen8 resources too, and versions do not move one way (`.ytd` goes 13 → 5 while
`.yft` goes 162 → 171, in `crates/io/or-io/src/container/resource/versions.rs`).
`.ydr`, `.ydd`, `.ytd`, `.yft` and `.ypt` have
separate Gen8 and Gen9 layouts, and OpenRage reads and writes both.

## Audio and video

A `.bik` video plays in the app with its sound, and an `.awc` stream plays, on every platform.
OpenRage decodes every audio and video format a shipped install uses, Bink 1 and Bink 2 pictures,
Bink sound and Legacy's XMA streams among them, in its own Rust, so no FFmpeg or other program
needs installing. A Bink 2 movie's pictures equal the game's own, plane for plane, in-loop filter
included, on every shipped movie of both editions.

## The mods overlay

A shipped archive can be copied into the install's `mods` overlay from its menu, or from the prompt
a refused write raises, and the app reads the copy in place of the original. When a title update has
changed the archive a copy was made from, the app names the copy in a warning, in the tree and in
Settings, and refreshing it lists what of yours it would discard before it writes. Settings ▸ Editing
▸ Mods overlay says whether the game will read the overlay, from the plugin and ASI loader beside its
executable, and names what to add and where; OpenRage downloads and installs nothing, and no run of
the game has confirmed a verdict. View ▸ Panels ▸ Mods lists the install's mods by kind (loose
files, a DLC pack, an ASI plugin or loader, a script mod) in a load order kept per install:
reordering two mods that write one path changes which one the app resolves it to, and Disable and
Enable move a mod's folder between `mods` and `mods_`. The order is not yet written into the overlay
the game reads. A project build can write an `.oiv`, and there is no `.oiv` installer.

## Plugins

A plugin is a WebAssembly module under the configuration directory's `plugins` folder
([plugins/README.md](../../plugins/README.md)). Its manifest names the capabilities it wants, the user
answers before its first run, and every refusal is shown. A plugin can read the open document's
fields and patch one, read the file's bytes and write bytes back through the edit gate, so a guest
that links `or-io` parses a `.ydd` itself, and, staying loaded, add panels drawn from the widgets its
guest describes, commands, settings and viewport overlays with picks. It cannot yet read another
archive's files.

## Red Dead Redemption 2

An install is found and its RPF8 archives open, a body compressed with Oodle Kraken included, which
`or-kraken` decodes. Its resources are another container, RSC8, which nothing here decodes yet, so the
matrix's other rows are empty for it.

## Earlier titles

`or-io` reads and writes the archive layouts of Table Tennis (RPF0), GTA IV (RPF2, RPF3 for its audio,
and the IMG3 image), Midnight Club: Los Angeles (RPF3), Max Payne 3 (RPF4), Red Dead Redemption
(RPF6), GTA III and Vice City (IMG1) and San Andreas (IMG2), through the calls RPF7 has, and nothing
inside them. An archive its title enciphers is refused by name until that title's key is supplied, and
the editor lists none of these games as a title.

## GTA VI

The table of contents of an RPF9 archive is enciphered, and OpenRage ships no key: the keys are read
from the user's own executable, and [GTA_VI_Modding.md](../guides/GTA_VI_Modding.md) gives the steps.
With them, an archive the editor opens is a folder of the Files tree, as a GTA V one is: a nested
archive is a folder in it, each row shows its entry's kind as the table states it, a double-click
opens an entry, Extract writes one out, and the Inspector shows a selected entry's row; there is no
separate archive panel. An entry is named where OpenRage holds a name that hashes to it, from your
names folder, the names you mined and harvested, kept through one cache in the configuration
folder, and the bundled words, and by its hash otherwise.
`openrage-cli rpf9` lists, finds, counts and extracts entries from the table, nested archives
included, an enciphered one deciphered, and `rpf9 tree-install` merges an install's paths in the
order the game mounts its archives. The install opens as a plain folder. Only without keys do the
editor and `openrage-cli rpf scan` fall back to finding the bodies an archive stores in the clear:
the archive then opens as its scan's list, in a tab of its own, with a message saying why, and the
folder's view offers the scan and the name harvest.

- **Read.** Texture packs (`.ttpf`) and dictionary textures, drawables, fragments, dictionaries of
  drawables and their skeletons, bounds, navigation cells, map placements and map types, clip
  dictionaries and the other animation bodies, cloth, a ped's hair grooms among it, PSO, RBM and RBF
  metadata, ped catalogues among it, text tables, AWC audio,
  which plays and exports as Ogg Opus, and a Bink movie's pictures and sound. A drawable is drawn in
  Wireframe, Solid and Material and goes out to glTF. An enciphered entry opens deciphered: a PSO or
  RBF in the metadata editor, text as text and a resource in its kind's view. A PSO's enciphered
  strings name the hash values it holds. A deciphered script body reads as a compiled program: its
  header, instructions and native calls through `openrage-cli script compiled`, and as source through
  `openrage-cli script decompile` and the app. `script compiled --set-int` and `--set-float` write a
  new body of the same length with a push's value changed; `--set-string` is refused, since no string
  text of the build's programs was found to replace.
- **Textures.** A texture stored plainly or as zlib reads as it is. One stored through the pack's
  entropy coder needs the coder's tables, which no file of the game holds. With a tables file you
  supply to `openrage-cli ttpf tables`, every mode reads through it, and a texture is called exact
  only on a condition. A BC6H or BC7 texture: no block of it speaks against the file, and
  `openrage-cli ttpf tables --check` has held the file's tables of its format to your packs. A BC1
  texture: the file and BC1's built-in tables give the same value for everything it reads. A BC3,
  BC4 or BC5 texture is never called exact, since no rule of those formats can show a wrong table.
  A file that fails a check is reported as not this coder's tables from then on, a listing or an
  export that counts a block against the file fails it for that pack and keeps that, and no read
  is ever a comparison with the pack's own blocks. A texture none of whose coded bodies was read
  says that its plain levels were read and its coded levels were not, and neither it nor one a
  coded body of which was not read is called exact.
  Without one, BC1 decodes through its built-in tables, which were fitted and which nothing
  confirms value for value: a BC1 texture read through them is marked as resting on the built-in
  tables and is not called exact. BC3, BC4, BC5, BC6H and BC7 decode through tables that
  `openrage-cli ttpf calibrate`, or the app's job, fits on your machine from the packs of the
  folder, and are refused by name before that. A texture read through fitted tables is an estimate
  and is marked calibrated wherever it is shown or exported
  ([the two sources](GTA_VI_Xbox_Build.md#textures-the-coder-and-its-tables)).
- **Shaded.** Shaded draws a pass of a GTA VI drawable with the game's own programs where both its
  vertex and its pixel program translate and bind, lit by the hour and the weather the install's
  time cycle states, and its button says how many programs were refused and why the first was.
  The sky, the exposure and every pass that is not a model's are the editor's own.
- **Weapons.** A weapon is a carrier model and a model for each part. Where the archive's names are
  harvested, the carrier is drawn as the first variant of each kind of part at its own level of
  detail, each with its root bone on the bone of the same name that the carrier or another part
  offers; with the viewport's Parts box unticked the carrier's own mesh is drawn. A part's material
  holds no colour map: the game computes a weapon's colour from data no model holds, so Material
  shades a part by its normal map alone.
- **Written.** Every write goes to a new file, never over what it read. A texture pack saves with a
  texture replaced, and an edit to a body saves as a loose file. Which values of a model, a bound, a
  map or a metadata body can be edited is narrower than for GTA V, and a refusal names what it cannot
  write. A new archive is of one of two kinds, and each write says which. A patched copy puts each
  changed body in its entry's place, chunk by chunk, and keeps the table byte-identical, so its
  signature still matches and no check read in the game's code rejects it; the game has not been run
  with one. In the app, a document opened from a row of an archive's tree saves into that row,
  and the tree's Save as… and File ▸ Save into a copy of the archive… write a patched copy when
  every change fits, a headerless resource's as well as any other row's; `openrage-cli rpf9 patch`
  writes one, and `rpf9 fits` says whether a replacement fits. A change fits only at the body's own
  length, and a changed chunk is deflated again with zlib's levels and then, on a chunk within a
  size cap, an optimal parse, which must fit the stored size the table states. On the one build
  measured, most unchanged chunks fit again, and an edit made by each of the text table, script,
  metadata, texture and model writers fitted and read back (`gta6_edit_to_patched_copy`, on the
  code of `e4d30d553`); a chunk past the cap fits only where zlib fits it.
  [GTA_VI_Modding.md](../guides/GTA_VI_Modding.md#an-edit-in-the-app-from-the-files-tree-to-a-patched-copy)
  walks one edit from the tree to a patched copy, gives the CLI's steps and a script's, and says
  [what fits today](../guides/GTA_VI_Modding.md#what-fits-today) with the measured figures.
  A rebuilt archive lays a table out again, which the game refuses because only its maker can sign
  one: `rpf9 replace`, `remove`, `rename`, `defrag`, `add` and `build`, `dlc create-gta6`, and the
  app's saves when a change does not fit: a row renamed, removed or added, a body whose length
  changes, a chunk whose new stream outgrows its stored size, or a row the patch writer refuses,
  each named in the message with why it kept the copy from being patched. A plain replace of an entry whose row describes a paged resource is refused by name, and
  `replace --resource` replaces it by its loose form.
- **Not done.** The game refuses every rebuilt archive, and no file OpenRage saved, a patched copy
  included, has been tried in the game. A compiled script,
  which `openrage-cli script` decompiles, names a native, a type or a member only where a supplied
  names list holds it, and comes out of an archive only with a key snapshot that holds content keys
  ([GTA_VI_Script_Decompiler.md](GTA_VI_Script_Decompiler.md)). A program the shader front end
  cannot translate or bind leaves its pass undrawn in Shaded, and no save file of the game has been
  opened: the save reader was written from a description of the layout.
  [GTA_VI_Xbox_Build.md](GTA_VI_Xbox_Build.md) has each reader, refusal and command.
