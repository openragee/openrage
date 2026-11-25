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

An RPF9 archive opens from its encrypted table of contents. It needs the user's key snapshot,
`gtavi_magic.dat` in the configuration folder or the file `OPENRAGE_GTA6_KEYS` names; a masked
snapshot also needs the executable `OPENRAGE_GTA6_EXE` names. Missing keys or an invalid table
produce an error, with no scan fallback. The Files panel lists the entries and loads a nested
archive's table when it is expanded, on a worker and within its containing entry's bounds.
In an opened folder, RPF9 archives expand inside the same Files tree as other archives. Resolved
paths form folders; unresolved hashes remain at the root of their owning archive. File icons use
the table's type or the entry's leading header, independently of the displayed name. Opening a
folder also checks entry headers at their table addresses: AWC containers show the audio icon and
`.awc`, and PSIN metadata shows `.psin`. This reads only the leading bytes, with a bounded input
budget for compressed or encrypted entries. Files adds a visual extension to recovered names and
hashes (`.ytd`, `.ydr`, `.rpf`, `.awc`, `.psin`, or `.bin` when no extension is identified), while keeping
the original name and table hash unchanged. Properties show the original entry index
and 64-bit hash. Opening an entry keeps its Files selection and breadcrumb trail. The archive's
contents are read-only, including tree edits. An entry whose table does not state
its class offers **Open as archive**, which checks only its stated position and extent.

The table view opens a file with **Open** or a double-click, and **Extract…** writes its decoded
bytes into a chosen folder, keeping existing files. Both actions use the same bounded reader.
The tree and its filter include the type indicated by the table; the selected entry's facts
separate that hint from a type detected in its decoded contents. Texture dictionaries open in
the texture preview, in memory and read-only, after their sections, every entry and every texture's
pixels have been validated. A partly unreadable dictionary is an error, not a partial preview.
Names stay as hashes where the vocabulary has no match; a detected type does not invent a filename.
Text files open as text and other decoded contents as read-only bytes; a resource extension never
selects a GTA V reader. Reopening an entry follows its archive indices through the tables again.
Neither type detection nor preview creates an extracted file or searches for content outside the entry.

Recognised `.ydr`, `.ydd` and `.yft` entries open with the GTA VI model reader. Their shader
texture bindings use 64-bit keys to find external `.ytd` entries in mounted RPF9 archives and
the configured GTA VI install. A bounded worker reads the tables, indexes texture names from
the dictionaries' virtual sections and opens matching dictionaries in memory
(`src/state/rpf9_texture_resolver.rs`). This resolution uses table addresses throughout, with
no content scan or extraction to disk. Textures that cannot be located or decoded remain
unavailable; opening a model does not imply that every texture or material is supported.

A flat, headerless GTA VI `.ydr` opened manually is recognised from its root and internal name,
then confirmed against a byte-identical entry in a mounted or configured RPF9 archive
(`src/state/rpf9_loose.rs`). The matching table supplies the virtual and physical section sizes;
the reader does not guess their boundary. Without that match, the file is refused with an
explanation. Files with an RSC7 header continue through the GTA V reader.

The reader supports sized raw entries on 24-byte rows. Chunked entries use mode 3 for 24-byte
rows, mode 2 for 20-byte resource rows, or mode 4 on either shape. Each descriptor gives the exact
stored and decoded sizes and selects stored bytes or zlib, including mixtures of both. A 20-byte
row skips the header slot, checks the chunks against the stored extent, and derives the decoded
size from the chunks. Compressed-resource rows without a chunk record describe a single raw
DEFLATE stream. Both row shapes support content-ciphered entries when their key and framing are
supported. Missing or inconsistent descriptors and unknown layouts are refused. Extraction writes
progressively and publishes a file only after successful decoding, including nested archives too
large for the in-memory byte view. Nested archives open as tables in the tree.
Extraction does not reconstruct a loose resource header around headerless
contents. These paths do not scan for content or recover from a failed decode. The
[gta6_resource_reads](../../crates/io/or-io/tests/gta6_resource_reads.rs) suite checks exact chunk
sizes, bounds, limits and decoding failures; app and CLI tests cover opening and extraction.
The app's install test `every_entry_in_the_selected_archive_opens_and_extracts_from_tables` walks
all tables in `OPENRAGE_GTA6_ARCHIVE`, opens files or nested tables and compares the extracted bytes. Run it
with the `all` test profile and the key environment variables above; it never scans the archive.
The app's `every_table_texture_dictionary_in_the_selected_archive_previews_in_memory` test
checks all texture dictionaries in that selected archive without writing their contents to disk.
The `selected_entries_across_five_archives_preview_and_reload_without_extracted_files` test uses
`OPENRAGE_GTA6_PREVIEW_INSTALL` and the same keys to check preview pixels, reopening and other types
in several archives. Both preview tests also use the `all` profile.

`openrage-cli rpf9 list` also lists from the table, and `rpf9 extract` uses the same strict entry
reader as the app (`--decrypt` enables ciphered entries). The explicit `openrage-cli rpf scan` command and
the World index retain their scanners and their existing body readers. The install opens as a
plain folder.

The [GTA VI modding guide](../guides/GTA_VI_Modding.md) describes obtaining keys from the user’s executable.
View ▸ Panels ▸ Archives also lists tables; the CLI can write a new archive and export PSO strings.

- **Read.** Texture packs (`.ttpf`) and dictionary textures, drawables, fragments, dictionaries of
  drawables and their skeletons, bounds, navigation cells, map placements and map types, clip
  dictionaries and the other animation bodies, cloth, a ped's hair grooms among it, PSO, RBM and RBF
  metadata, ped catalogues among it, text tables, AWC audio,
  which plays and exports as Ogg Opus, and a Bink movie's pictures and sound. A drawable is drawn in
  Wireframe, Solid and Material and goes out to glTF. An enciphered entry opens deciphered: a PSO or
  RBF in the metadata editor, text as text and a resource in its kind's view. A PSO's enciphered
  strings name the hash values it holds. A deciphered script body reads as a compiled program, its
  header, instructions and native calls, through `openrage-cli script compiled`.
- **Textures.** A texture stored plainly or as zlib reads as it is. One stored through the pack's
  entropy coder decodes exactly as BC1; as BC3, BC4, BC5, BC6H or BC7 it decodes through tables
  that `openrage-cli ttpf calibrate`, or the app's job, fits on your machine from the packs of the
  folder, and is refused by name before that. Such a texture is an estimate and is marked
  calibrated wherever it is shown or exported.
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
- **Written.** A texture pack saves with a texture replaced, and an edit to a scanned body saves as a
  loose file. Which values of a model, a bound, a map or a metadata body can be edited is narrower than
  for GTA V, and a refusal names what it cannot write. `openrage-cli rpf9 copy`, `replace`, `remove`,
  `rename` and `defrag` write a new archive from an existing one, never over it; a plain replace of
  an entry whose row describes a paged resource is refused by name, and `replace --resource` replaces
  it by its loose form. The app's File ▸ Save into a copy of the archive… writes a document's edits
  in place of its entry, in a new copy of the archive it was read from.
- **Not done.** The game refuses every archive OpenRage writes with a changed table, because only its
  maker can sign a table, and no file OpenRage saved has been tried in the game. A script is not
  decompiled from its compiled body. A program the shader front end cannot translate or bind leaves
  its pass undrawn in Shaded, and no save file of the game has been opened: the save reader was
  written from a description of the layout.
  [GTA_VI_Xbox_Build.md](GTA_VI_Xbox_Build.md) has each reader, refusal and command.
