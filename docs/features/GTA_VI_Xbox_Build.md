# GTA VI Xbox build

What OpenRage reads, edits and saves of a GTA VI Xbox build, what it refuses and why, and how far each part was checked against a real build.

GTA VI is a title of its own (`crates/title/or-title/src/game/profiles.rs`) with no declared format: nothing of it is a `#[record]` declaration, because there is no source to name a field by. Its readers are ordinary code in `crates/io/or-io/src/gta6/`, behind `or-io`'s `gta6` feature, and a name there says the role a field was measured to play. The archive is `crates/io/or-io/src/container/rpf/rpf9.rs` and the files beside it. The layouts themselves are in the gitignored docs/extra/ tier and are not repeated here.

## An archive is scanned, never listed

An RPF9 archive's header is in the clear; its table of contents and its names are encrypted with a key OpenRage does not have. So the archive is refused as an archive, and read another way: `crates/io/or-io/src/container/rpf/rpf9_scan.rs` passes over the file and finds the bodies stored in the clear by their own bytes.

- **A find has an offset and no name or path.** Every command and every view addresses a body by its offset in its archive. A body's end is stated only where its bytes state it or where the measured rule has no exception.
- **A find has a kind and a family.** The kind is what the bytes are (a nested archive, a run of zlib chunks, a stored resource, a movie, an audio container, an effect container, and a few more); the family is the word a person filters by: models, dictionaries, collision, navigation, animations, metadata, text, audio, movies, shaders, archives, other and unknown.
- **A headerless resource has a role.** A resource with no header is placed in a family of layouts by the shape of its first words (`Rpf9Role`): a drawable, a fragment, a bound, a dictionary, a navigation cell, a particle effects list and the others the enum lists. A resource no layout places opens as its bytes, under the reason.
- **A whole scan is kept.** A scan that read an archive to its end is kept in the configuration folder (`src/state/rpf9_kept.rs`), and the archive is listed from it while its size and modification time stand.

The app opens a GTA VI folder as a plain folder, and an archive as the list of what a scan finds, a slice at a time on a worker (`src/state/rpf9_scan.rs`). Above the folder's Files view, a scan of every archive tallies the bodies by kind across the folder (`src/state/gta6_install_scan.rs`), and a click opens one.

⚠ A scan is not the table. A body stored encrypted, or one whose bytes fit no detector, is not found, and nothing a scan finds can be written back into its archive: every save writes a file of its own.

## What is read

Each row is a family of bodies, the module that reads it, and where the app shows it. "Edits" names what a save writes; a blank cell is read only.

| Bodies | Read by | Shown as | Edits |
|---|---|---|---|
| Texture packs (`.ttpf`) | `crates/io/or-io/src/gta6/ttpf.rs` | a pack document, its textures in the Gallery and the texture preview | a texture replaced, resized, re-encoded; one slice of a cube, an array or a volume replaced |
| Drawables, fragments, bounds, dictionaries, cloth, morph targets | `crates/io/or-io/src/gta6/headerless/` | a model document in the viewport, with Outline and inspector as a GTA V model has | constants, texture keys, materials, bones, geometry from glTF, embedded and dictionary textures, bound vertices, masses and materials of collision |
| Navigation cells | `crates/io/or-io/src/gta6/headerless/navmesh.rs` | the navigation mesh view | a polygon's flags; a polygon disabled or deleted |
| Particle effects lists | `crates/io/or-io/src/gta6/headerless/particles.rs` | the Particle workspace | rule scalars, flags and keyframed values, in place |
| Animations: clip dictionaries, animation sets, motion dictionaries, expression sets | `crates/io/or-io/src/gta6/anim.rs` | an animation document with its player and timeline | keys, a clip's window, rate and name, tracks added, removed or held; an expression track's id |
| Anim scenes | `crates/io/or-io/src/gta6/anim_scene.rs` | a timeline over the metadata view | |
| Ped catalogues | `crates/io/or-io/src/gta6/ped_catalogue.rs` | a metadata document; `rpf scan-open` prints its kits | values, as any metadata body |
| Map placement and archetypes | `crates/io/or-io/src/gta6/map.rs` | the Map screen | an entity moved, turned, scaled or retyped |
| Lights, the lane graph, world census | `crates/io/or-io/src/gta6/map_lights.rs`, `crates/io/or-io/src/gta6/map_lanes.rs`, `crates/io/or-io/src/gta6/map_world.rs` | overlays of the Map screen | |
| Other metadata bodies (PSO, RBM, RBF0) | `or_io::meta`, and `crates/io/or-io/src/gta6/ui_tree.rs` for an RBF0 tree | a metadata document | values, saved as a file of its own |
| Mission state tables | `crates/io/or-io/src/gta6/mission_tables.rs` | rows in the outline of the script body's metadata document | |
| Text tables | `crates/io/or-io/src/gta6/text.rs` | a text document | texts |
| Audio game data, radio, dialogue | `crates/io/or-io/src/gta6/audio_data.rs`, `crates/io/or-io/src/gta6/audio_radio.rs`, `crates/io/or-io/src/gta6/dialogue.rs` | an item list, and stations and lines with a play button | |
| Audio containers | `or_io::awc`, decoded by `or-media` | the audio view | a stream replaced from a WAV, for the codecs `or-media` encodes |
| Movies | `crates/io/or-media/src/bik.rs` | the video preview, picture and sound | |
| Effect containers and their programs | `or_io::gta5::shaders::sgd5`, `crates/tools/or-shader-front/src/rdna/` | an effect document; Shaded where the programs translate | |
| MoVE networks, height maps and the other bodies a head names | `crates/io/or-io/src/gta6/move_network.rs`, `crates/io/or-io/src/gta6/other_bodies.rs` | their own views or the facts the reader reads | a height map's grid |
| Saves | `crates/io/or-io/src/gta6/save.rs` | a save document | a story save's values |

A ped's hair is among the cloth: a groom is stored as a cloth record whose root holds guide strands over a root mesh, skinned to the ped's bones, one record a level of detail (`crates/io/or-io/src/gta6/headerless/cloth.rs`). It is drawn as its strands and the sheet over their roots; the strands the game grows between the guides are not drawn.

A ped catalogue is a headerless metadata body of two lists of kits. A kit of the first states the assets of one part of a ped, a state at a time: a drawable and its albedo, normal and material maps, each by the 64-bit hash of its name. A kit of the second names kits and a state of each. The reader finds a member by its name hash in the body's own table and leaves the members it does not follow in the generic tree.

A reader names what it does not read. Where the meaning of a field is not known, the reader keeps the field as it was read and the view shows it under its stored form or its hash; the module headers under `crates/io/or-io/src/gta6/headerless/` list those fields file by file.

## How an edit is saved

Nothing is written into an archive. A save of a scanned body writes a **loose resource**, a file that a read at offset 0 returns as the sections it was written from (`crates/io/or-io/src/container/rpf/rpf9_write.rs`). Save As makes one from a scanned body, and Save writes over a loose resource that was opened from disk (`src/state/gta6_resource_save.rs`).

Edits come in two kinds, and the difference decides how much can go wrong:

- **An edit that moves nothing** is written over the bytes its reader read: a constant, a texture key, a bone's transform, a key of a channel, a particle value, a texture in its own size, format and mips. The body is read again on the save's worker and the values shown are written in place.
- **An edit that changes a length** lays out part of a section anew: a material that gains a parameter, a bone renamed, a geometry that outgrows its buffer, a navigation polygon deleted, a dictionary texture resized. `crates/io/or-io/src/gta6/map_grow.rs` adds and removes a map's entities the same way, and no view or command offers it yet. `crates/io/or-io/src/container/rpf/rpf9_grow.rs` moves every pointer at or past the edit. What a pointer is comes from a typed reader, so a word that looks like a pointer and that no reader names refuses the edit.

A pack saves as a pack: every row of a texture left alone is carried as the file stores it, and a replaced texture is written as zlib rows of the sizes its entry states (`crates/io/or-io/src/gta6/ttpf_write.rs`). A texture given another size, format or mip count moves every texture after it (`crates/io/or-io/src/gta6/ttpf_relayout.rs`).

⚠ No written file has been seen to load in the game. A loose resource and a saved pack open again in OpenRage as what was written, which is a round trip through one description and proves nothing about the game's reader.

## What is refused, and why

| Refused | Why |
|---|---|
| Listing an archive by name, and writing an entry back into one | the table and the names are encrypted and OpenRage has no key |
| Recomputing a PSO body's checksum | the algorithm is not known; an edited PSO is saved with the checksum it was read with, and the app says the game may refuse it (`src/state/gta6_meta_save.rs`) |
| Writing a changed save record | its header holds a checksum of an unknown algorithm, so a record is read and never written |
| A packed copy of a save | its body is compressed or encrypted; it is refused by the title and slot its clear head states |
| A pack body of BC3, BC4, BC5, BC6H or BC7 blocks stored through the coder, before a calibration | the coder's tables for those modes are in no file ([below](#textures-the-coder-and-calibration)) |
| A program the native front end cannot lower, or whose resources it cannot bind | each refusal has a name; a pass with a refused program is not drawn in Shaded ([below](#shaders)) |
| An edit that would renumber what another body names | a navigation polygon another cell links to, a map other maps name as parent: the other bodies are saved apart |
| An entity retyped to an archetype its map does not already name | the archetype word is a namespaced hash and the writer refuses one the body did not hold |

Three words are kept as read because no rule derives them: a clip block's hash, a geometry's content digest and a PSO's checksum. Whether the game checks them is not known.

## Textures, the coder and calibration

A pack is a header, a table of rows, a directory of textures and bodies of tiled texture memory. A texture is keyed by the 64-bit hash of its lower-cased name, and the pack does not hold the name.

A body is stored plainly, as zlib, or through **BCPack**, the console's entropy coder over BC blocks (`crates/io/or-io/src/gta6/bcpack.rs`). The coder's lanes of every mode decode. Turning a lane's symbols back into blocks needs tables that are constants of the coder and are in no stream:

- **BC1 decodes exactly.** Its tables come with the port of bcpack-rs. One preset slot has no known value, and a body that reads it is treated as the modes below are.
- **BC3, BC4, BC5, BC6H and BC7 decode through tables fitted on the user's machine.** No texture is stored both coded and plain, so `crates/io/or-io/src/gta6/bcpack/calibrate.rs` fits each table against the plain mip levels beside a coded one, over the packs of a folder, in passes. BC6H and BC7 each read two presets, of endpoints and of indices, which `crates/io/or-io/src/gta6/bcpack/bc6h_fit.rs` and `crates/io/or-io/src/gta6/bcpack/bptc_fit.rs` fit.
- **A texture whose plain levels are not one another averaged down gives no target.** A map filtered anew for each level is such a texture; its coded levels are drawn through what the other textures fit, and vote only what those left open, against the pixels across each block's edges.
- **A signed BC6H texture is drawn through the tables the unsigned textures fit.** No signed texture has a plain level beside a coded one, so a calibration reads one only for what needs no target: the range of its ends, read as signed, and the pixels across its blocks' edges. What checks its decode needs no pixel either: an encoder's difference never takes an end out of its base's range, and the blocks of a signed body kept to that through tables fitted without them.

**A calibrated texture is an estimate.** It is the picture the fitted tables give, not the picture the game decodes, and it can be wrong in any block drawn through a slot or symbol the fit did not solve. `Decode` counts both: the blocks rebuilt through fitted tables, and those of them drawn through an unsolved slot. The inspector, the Gallery, `ttpf info` and an export each say "calibrated" with that share, and a save never writes an estimate: it carries the coded bodies as the file stores them.

`openrage-cli ttpf calibrate <folder>` and the app's job (`src/state/gta6_bcpack_calibration.rs`) fit the tables and keep them in the configuration folder for the packs they read, under a name the packs' names and lengths give, so a game moved to another folder keeps its calibration. The app offers the calibration in a pack's list while the game has none, until it is done or declined. A run that is stopped or cut short keeps where it stood after each pass, and the next run of the same packs goes on from there. A slot is fitted only from the packs read, so the folder named should hold every pack of the game. The table the command prints says how far each mode's coded mips are from the plain mips beside them, in 8-bit steps and for BC6H in stops, which is the only measure of a fit there is. A pass reads each texture once, on up to 16 threads that each hold one texture; `--coverage` then decodes every coded body of the folder once and counts, by mode, the textures that read exactly, as estimates, or not at all, which takes longer than the fit.

## Names, naming rules and use labels

Nothing in an archive names a body, and a pack names no texture. Names come from what the bodies spell.

**The harvest.** `crates/io/or-io/src/gta6/names.rs` reads the start of each body for printable names: the names resources state of themselves and of each other, and plain ones. Each name is hashed both ways, 32-bit and 64-bit (`crates/io/or-io/src/gta6/hash.rs`), into a `Vocabulary` kept in the configuration folder. From then on a hash read anywhere is shown by its name, in the CLI and in the app. The harvest also keeps the name each drawable and fragment states of itself, by its offset, which a scan's list shows beside the row. `openrage-cli rpf scan-names` runs it, and the app runs it as a job (`src/state/gta6_names.rs`, `src/state/gta6_install_harvest.rs`).

**Map names.** A map body states its name as two hashes, and few bodies spell a map's name. The names of the maps that go with a named map follow from it: its occlusion and level-of-detail maps, its instance maps by range and detail, numbered from 0, and its area's imposter maps (`crates/io/or-io/src/gta6/map_names.rs`). As with a texture, a rule makes candidates and a candidate names a map only where it hashes to the 64-bit word a body states. `rpf scan-maps` ends with how many maps, archetypes and entities the kept names name and how many maps more the rules name, beside the same count from the names reversed, which is what chance gives; `--names` keeps the names the rules gave with the others.

**A list of names.** `rpf scan-names --names <file>` holds each line of a file the user supplies as a plain name of the vocabulary, so a hash it names is shown by it wherever the vocabulary is asked. No list ships with OpenRage. ⚠ A list found by searching for hashes can hold a name that hashes right and is wrong, and a 32-bit hash two held names share names nothing.

**Member names.** A metadata body states its own layout and names nothing: a structure, a member, an enum and its values are hashes. `openrage-cli meta snapshot --par-tree <dump.json>` turns a parser dump, the JSON a viewer of the executable's reflection tables carries, into a schema snapshot of GTA V's format (`crates/io/or-io/src/meta/par_tree.rs`), kept beside GTA V's in the shared store and never in the tree. The editor and the CLI read it for its names alone, and only for a GTA VI body: the readers and writers of GTA VI's metadata hold `hash_index::title_names()` while they work, and a GTA V document is named as if the snapshot were not there, because the two games' hashes are unrelated. A GTA VI body opens with the names the dump has in the clear and `hash_XXXXXXXX` for the rest. ⚠ A snapshot holds no offset: a dump's layout is the executable's, and a body may state another, so the body's own table is what is read.

**The naming rules.** A texture's name follows from the name of the drawable that uses it: the drawable's stem, then a map kind, a level-of-detail token swapped, a numbered variant, the stem's first or last tokens, or a ped component's pattern (`Rule` in `crates/io/or-io/src/gta6/texture_names.rs`). The rules make candidates, and a candidate names a key only where it hashes to one, so a rule can miss and cannot name wrongly short of a hash collision. Nothing there is a list of names. A file of names the user picks is read the same way.

**The use label.** A texture no name is found for is still used: the harvest records, for each texture key a model's material binds, the first model that binds it, the shader parameter that binds it there, and how many models bind it. The label is that model and parameter, as in "used by a model as its diffuse texture, and by two models more". It is not a name, since it hashes to no key; it tells a person what a nameless texture is for, and the pack's list filters by it.

## Shaders

GTA VI's Xbox programs ship as native GPU code with no bitcode. Two front ends in `or-shader-front` turn a program into naga IR, and neither runs the game's compiler:

- `dxil` reads a container that holds bitcode.
- `rdna` reads native code: `crates/tools/or-shader-front/src/rdna/decode.rs` decodes the instruction words, `crates/tools/or-shader-front/src/rdna/cfg.rs` cuts them into a control-flow graph, `crates/tools/or-shader-front/src/rdna/sym.rs` follows where each scalar register's value came from, which is the only way a program names its resources, and `crates/tools/or-shader-front/src/rdna/to_naga.rs` lowers a pixel, vertex, mesh or compute program to a module. Everything it does not model is refused by name (`NotTranslated`).
- `crates/tools/or-shader-front/src/rdna_bind/mod.rs` joins each resource of a translated module to the binding record its package lists and to the name the effect container's table gives that register, and refuses, by name (`Unbound`), a program the renderer has nothing to give: one that fetches its vertices from buffers the game fills, reads numbers the game passes with each draw, or picks a resource by a number the game sets.

The app translates the vertex and pixel programs a drawable's effects run, on a worker, once a session, into a pack held in memory (`src/state/gta6_shaders.rs`), and Shaded draws a pass only when both its programs translate and bind (`src/render/gta6_shaded.rs`). The Shaded button says how many programs of the drawable's passes are refused and why the first is. The other shading modes need no program.

The light of Shaded follows the install where its common archive is found: `src/state/gta6_timecycle.rs` reads the time table and the weather cycles among that archive's XML bodies with GTA V's reader, and `src/render/viewport/gta6_day.rs` puts the sun and the moon where the hour puts them, lets one of them light as their heights say, and takes the two samples of the weather's cycle that stand for the sun's angle. The cycle states the light's illuminance, its fade, its colour and what multiplies the sky's ambient; the Shaded line says when a model is lit this way. ⚠ Two things are still the editor's own: the exposure, which the game meters from each frame on the GPU, stands as one constant for sunlight and one for moonlight, and the colour of the sky and of the ground is the built-in sky's, since the game computes it from its atmosphere.

Shaded for a GTA VI drawable is therefore partial by construction, and this document states no share: `rpf scan-rdna` and `rpf scan-rdna-bind` count, for an effect container, how many programs decode, translate and bind, and what stopped the rest.

What a translated program is on a device is checked by `or-shader-tool rdna-devices` (the `device-check` feature), which builds every program and every pass's pair of a container into a pipeline on each adapter of the machine, in worker processes a driver's crash ends one of. `--draw` then draws with each pipeline, a stand-in at every binding and no vertex, so the device checks the pipeline against all it is drawn with and runs none of it: fed stand-ins, a program's loops need not end. `--msl` has naga write each as Metal's language with no device, and `rdna-devices --help` says how another machine adds its adapters to the report. What a reader may rely on:

- **Vulkan and Direct3D with DXC are the two backends the translated passes are built and drawn with.** Metal is written and validated as text only; no Metal device has built one.
- **Direct3D needs DXC.** FXC refuses programs that are valid (a gradient inside a loop, a barrier in a branch) and has no 64-bit integers. `or_renderer::instance` gives every instance the `dxcompiler.dll` beside the program, else one on `PATH`, else a Windows SDK's, and warns when it falls back to FXC; the app then says so in a notice when it starts, with what FXC loses and the two ways out. ⚠ A package for Windows ships neither `dxcompiler.dll` nor `dxil.dll`, so a machine with no Windows SDK compiles with FXC until a person puts the two files beside the program or starts it with `WGPU_BACKEND=vulkan`.
- **A pass that updates a texture of 64-bit texels is not built on Direct3D**: wgpu offers the feature there and refuses the texture, which `instance::updates_64_bit_textures` asks the adapter about.
- **OpenGL is not supported**: the shading passes index a table of textures at a computed place, which it cannot.
- ⚠ **One integrated adapter's Direct3D driver ends the process on some large mesh passes.** The check passes over them by name on that adapter (`--skip-crashed`); the app has no such list, and reaches Direct3D on such a machine only when asked, since Vulkan is taken first.
- A compute program is built and never run by the editor, and the check runs none either.

## Saves and mission tables

`crates/io/or-io/src/gta6/save.rs` knows three containers. A story save is an index over one JSON text; it opens as an owned value, a value is read or replaced by a path of steps, and it saves. A record is a header before a payload and is read. A packed copy's clear head is read and its body refused.

⚠ **The save reader was written from a description of the layout alone.** No GTA VI save was at hand, so no real save has been opened by it; where its comments say "measured", the measurement is the description's, not OpenRage's. Its tests build their saves by hand from the same description, so they show that the reader and the writer agree with each other and nothing more.

The mission state tables are the opposite case. An inflated body of a script archive is a metadata document in the table form, and a state table is one of its blocks, found by the body's own block table and type table and never by a scan of its bytes (`crates/io/or-io/src/gta6/mission_tables.rs`). They are read only: what orders the states is not proven.

## Text, radio and dialogue

A text table is a map from a label's hash to one text or to the grammatical variants of one. A written table opens as the same table and is not byte-identical to the game's, because the layout the game's writer chose is not known. A table states no language: `crates/io/or-io/src/gta6/text_language.rs` takes it from the order in which a change set registers the text archives, which holds only while the file holds as many as are registered.

Radio and dialogue are joins across bodies, all read only. A station names track lists, a track names a streamed sound by key, and a streamed sound names its audio streams (`crates/io/or-io/src/gta6/audio_radio.rs`). A dialogue line's label is its key in the text tables and its place among a key's labels numbers its audio stream (`crates/io/or-io/src/gta6/dialogue.rs`); the pairs are found by their shape in a body's bytes, not through the body's structure, and a line's number is settled from the streams that exist. `src/state/gta6_dialogue.rs` indexes the open scans so that a line plays with its subtitle in the language picked.

## What was checked against a build

- **Held by a suite.** `crates/io/or-io-verify/tests/ttpf.rs` reads every pack of a GTA VI folder named by `OPENRAGE_GAME_ROOT` and requires each head to hold to the container model and each texture to read or be refused by name. It is install-gated, so a run without the folder is a skip and not a pass.
- **Checked by hand with the CLI.** The other readers were written against the owner's build and re-checked with commands that can fail: `rpf scan-relayout` (the loose-resource writer's rules against stored resources), `rpf scan-particles --validate` (an in-place write gives a section back unchanged), `rpf scan-maps --check` (level-of-detail links resolve), `ttpf plan` (a save's rows against the file's). No suite in the tree runs them, and no measured figure of a build is committed.
- **Tested on inputs built by hand only.** Every unit test under `crates/io/or-io/src/gta6/` and `crates/tools/or-shader-front/src/rdna/` assembles its input from the layout; none reads a game file. Such a test passes when reader and writer are wrong the same way.
- **Not checked against real bytes.** The saves, as above.
- **Not checked in the game.** Anything written.

## Commands

Every command is `openrage-cli` and a row below; `openrage-cli rpf` and `openrage-cli ttpf` with no argument print the full usage. An offset is where a scan found the body.

| Command | What it does |
|---|---|
| `rpf scan` | lists the bodies a scan finds in an archive, or those that start in a range |
| `rpf list`, `rpf extract` | on an RPF9 archive, the scan's list by family, and the bodies extracted as stored, inflated or converted (textures as PNG, audio as `awc export` writes it, a model as glTF, metadata as XML) |
| `rpf scan-install` | scans every archive under a folder and tallies the bodies by kind |
| `rpf scan-body` | writes one body to a file: stored, as XML, as GTA V's text table, or rewritten |
| `rpf scan-open` | reads one body through the reader that fits and prints what it states |
| `rpf scan-extract` | writes every body of a kind, a family or a role to a folder |
| `rpf scan-export`, `rpf scan-import` | a drawable, fragment or dictionary entry to glTF, and its geometries replaced from one |
| `rpf scan-texture` | a dictionary's texture replaced, re-encoded, resized or added |
| `rpf scan-anim` | an animation body as JSON, or posed on a skeleton |
| `rpf scan-expressions` | an expression set's tracks, and a track's id rewritten |
| `rpf scan-particles` | a particle effects list's rules; `--validate` checks its in-place writer |
| `rpf scan-relayout` | whether the loose-resource writer derives each stored resource's layout |
| `rpf scan-maps` | every map body's structure, and the share of maps, archetypes and entities the kept names name; `--list`, `--check`, `--extents`, and `--names` to keep the map names the rules derive |
| `rpf scan-census` | bodies opened through `rpf scan-open`, a process each: the metadata rows, or with `--kind` those of a kind, a family or all, but those `--not` names, at most `--sample` of each row, `--jobs` at a time; how many read and were refused per row, the refusals ranked, and the root type of each metadata document |
| `rpf scan-world` | a census of an archive's world data, and the lane graph joined |
| `rpf scan-lights` | the lights of one map body, each where it stands in the world |
| `rpf scan-names` | harvests names into the kept vocabulary; `--names` adds the lines of a file of names the user supplies; `--pack` counts the texture keys they name |
| `rpf scan-radio` | the radio stations, their track lists and the streams they play |
| `rpf scan-dialogue` | the dialogue lines with their audio streams and texts, as a table `awc export --dialogue` reads |
| `rpf scan-shaders` | how every program of an effect container translates, by outcome and refusal |
| `rpf scan-rdna` | the same through the native front end alone, by stage, run by worker processes held to a memory ceiling, so a program that crashes, runs out of time or of memory is an outcome; `--json` writes a record a block, `--backends` writes each valid module with every shader writer, `--stage` and `--reason` list a class of blocks, `--program` prints one disassembly and its WGSL |
| `rpf scan-rdna-diff` | what changed between two `--json` runs: the outcomes that moved, the reasons that shifted by stage, the translations that got slower |
| `rpf scan-rdna-bind` | how many passes translate in both stages, join every resource and bind |
| `rpf scan-effect-use` | the drawables of an archive by the effects they name: how many each effect's own passes draw in Shaded, and why the rest draw as Material |
| `ttpf info`, `ttpf export`, `ttpf replace`, `ttpf plan` | a pack's textures listed, exported, one replaced into a new pack, and a save planned against the file's table |
| `ttpf calibrate` | fits the coder's tables from the packs of a folder; `--coverage` then counts how every coded texture reads |
| `gta6-save info`, `get`, `set`, `key` | what a save states, a value of a story save read or replaced into a new file, and the key a name hashes to |
| `gta6-mission` | the mission state tables of an inflated script body |
| `navmesh scan`, `navmesh disable`, `navmesh delete` | a census of an archive's navigation cells, and a polygon cut off or deleted in a loose cell |
| `awc export`, `awc decode-check` | a GTA VI audio container's streams written out, and every Opus stream decoded in memory |

Where each command lives, and the modules behind it, is in [Crate_Reference.md](../reference/Crate_Reference.md#tool-crates); what the tree ports for this work is in [LICENSE-third-party.md](../../LICENSE-third-party.md).
