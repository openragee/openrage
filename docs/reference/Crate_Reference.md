# Crate reference

Where each crate and each part of the app sits, what it is for, the rules
between them, and the traps each one carries.

This is the tree as it stands. GTA V's formats are declared in `or-io`, and
the design is [OR_IO_Specification.md](../direction/OR_IO_Specification.md); what is left
of the programme that built it is [R66](../direction/Roadmap.md#r66-the-or-io-crate). Building, testing, lanes and CI are
[Development_Workflow.md](../guides/Development_Workflow.md).

## Layers

A crate's directory names its layer; the members are the root `Cargo.toml`'s
list.

| Layer | Directory | Crates | Holds |
|---|---|---|---|
| foundation | `crates/foundation/` | or-job, or-test-support, or-transport | building blocks that know no file format |
| io | `crates/io/` | or-io, or-io-macros, or-io-verify, or-kraken, or-media | GTA V's containers, descriptions, canonical writer and `#[record]` declarations, and the decoders for the audio and video in them |
| title | `crates/title/` | or-title | where the bytes live: editions, archives, DLC order, the VFS |
| domain | `crates/domain/` | or-addonped, or-content, or-depends, or-editor, or-interchange, or-maps, or-particle, or-particles, or-plugin, or-project, or-source, or-wardrobe, or-weapons, or-world | the models of what is edited |
| runtime | `crates/runtime/` | or-anim, or-simulation, or-renderer | what is evaluated, simulated or drawn |
| tools | `crates/tools/` | or-cli, or-shader-front, or-shader-tool, xtask | programs over the lower layers, and the repository's own tasks |
| app | `src/` | openrage | the desktop editor and the `openrage-cli` binary |

Dependencies run one way: foundation → io → title → domain → runtime → app. Tools sit outside the chain, and only the app depends on them. Each crate
imports the layer it needs by name and there is no facade crate, so a
wrong-way edge shows in the diff; `cargo tree -p <crate> -e normal --depth 1`
lists the edges.

- **Placement.** Code that needs to know where a file lives goes in
  `or-title`; code that needs only the bytes goes in `or-io`. Editing
  code goes below the app when a second application would want it; the YPT
  panels do not qualify.
- **Absent edges.** `or-anim` → `or-particle` (the previews share the clock in
  `or-transport`);
  and any dev-dependency
  pointing up, which is why `crates/title/or-title/tests/adding_a_title.rs`
  stops one call short of `or_editor::Summary`.
- **Edges that look wrong.** None points up the chain since `or-test-support`
  dropped `or-io` (P64, #334). One tool depends on another: `or-cli` on `or-shader-front`,
  whose Rust front ends translate a GTA VI program for `rpf scan-shaders` and for the app's Shaded. The app depends on `or-cli` (its
  `or_cli::run` is all of `src/cli_main.rs`; `openrage-cli photo` draws with the
  editor's renderer, so it starts the `openrage` beside it as `openrage --photo`,
  `src/photo.rs`, rather than `or-cli` depending on a renderer) and on `or-shader-tool`, which
  `src/state/game_pack.rs` drives so that no rendering crate depends on a
  build tool. The app depends on every domain crate, `or-wardrobe` among them: the Wardrobe
  workspace plans and edits clothing through it (`src/state/workbench/wardrobe.rs`), and it reads
  `or-content` for the kinds of content to plan, adopt and start, so that it and `openrage-cli`
  list them once. It embeds `or-editor`,
  `or-particle` and `or-plugin`, and shares `or-depends`, `or-interchange`,
  `or-world` and `or-project` with `openrage-cli`; it reads `or-project` for the
  keys of its project file and for the plan, the build and the clean of the
  Project workspace, where the CLI reads it for `openrage-cli project`. The app
  and `openrage-cli` both depend on `or-source`: `src/state/workbench/cook.rs`
  recooks a saved source with it, `src/state/workbench/scan.rs` finds the source
  folders with it, `src/state/workbench/build_cooker.rs` and `openrage-cli project build`
  hand a build the cook (`or_source::cook::cook_source`) and what each folder cooks to
  (`or_source::cook::describe`), and `crates/tools/or-cli/src/import.rs` makes sources of game
  files with it for `openrage-cli project import`, and `src/state/workbench/imports.rs`
  makes them from the app's Import game files… and Import into project; `crates/tools/or-cli/src/source.rs` and
  `src/state/workbench/geometry.rs` put a model into a source, and `src/state/workbench/originals.rs` checks a
  source against the game with `or_title::install::read_virtual_file`.

## Foundation crates

**or-job.** `Cancel`, `Progress` and `or_job::run`, the loop both
`openrage-cli` and the editor run a bulk job through. A per-item failure is
counted and never propagated, and an interrupt is checked between items.
Progress is counters a second thread reads rather than a channel, because a
frame needs only the current numbers; a job that is no loop, such as the
editor's defragment or overlay copy, publishes its own with `Progress::reached`.

**or-test-support.** Corpus discovery (`Corpus`), target inference and the
skip ledger for every suite; never hand-roll a `read_dir` over fixtures. A query
without a target returns Gen8 files only, which matters because a Gen9 drawable
read through a Gen8 reader returns plausible zeros rather than an error.
Staging a corpus is `crates/io/or-io-verify/tests/fixtures/README.md`.

**or-transport** is the previews' shared playback clock.

## IO crates

**or-io.** GTA V's file formats: the RSC7 and RPF7 containers, the description
and canonical writer, and the declarations `#[record]` turns into owned values
and views. [OR_IO_Specification.md](../direction/OR_IO_Specification.md) owns its design and
[module tree](../direction/OR_IO_Specification.md#module-organisation), including the
description model in `desc`
([descriptions](../direction/OR_IO_Specification.md#descriptions)).
R66c folded or-rage-core, or-rage-types,
or-pack-build, or-pack-io, or-container and all of or-format-api except `names`,
which went with that crate in P63 (#335), into its modules; comments and history still use those names. Its
`bc7-encode` feature pulls in `intel_tex_2`, the workspace's one native link,
so CI builds it in its own step, and `or-source` turns it on for itself. `or_io::dds`, `or_io::awc`, `or_io::rel` and
`or_io::gfx` are ordinary code, not declarations, each behind a feature of its
name; `gfx` keeps a Scaleform movie's tags as bytes and converts it to SWF and
back. The binary world tables are ordinary code behind no feature:
`or_io::height_map`, `or_io::distant_lights`, `or_io::water_map`,
`or_io::world_blanket` and `or_io::nav_precalc`, and so are the streaming cache
files, `metadata.dat` and the audio sector grid (`or_io::cache_loader`,
`or_io::social_club_metadata`, `or_io::audio_world_sectors`) and the `.mrf` move
networks (`or_io::mrf`). The level's `water.xml` is ordinary code beside the other data
files, `or_io::gta5::data::water`: `CWaterData` types its water, calming and wave quads,
keeps every other element and attribute of the document for the save, and answers the
height of the water over a point. `gta5::script::decompile`
turns a script's bytecode into C-like source over the same reader as its
disassembler.

`or_io::meta`, behind the `meta` feature, is PSO, RBM, RBF0 and XML metadata
(`.ymt`, `.meta`, `.ymap`, `.ytyp`) in one `MetaInstance` tree. A writer
encodes against the file's own type table (`binary::layout::MetaLayout`) or one
`binary::synth` derives, and RBM round-trips only its decompressed payload.
`.ymt` is three formats, so dispatch on the leading magic. XML returns through
`or_io::meta::canonical`, a fixed point from the second cycle, never equality
with the original. A clean checkout has no schema snapshot: it spells a name it
cannot resolve `hash_XXXXXXXX` and imports PSO and RBM against the type tables
documents carry (`crates/io/or-io/src/meta/README.md`). XML whose root type
neither the snapshot nor the library declares reads as the tree its elements
state (`instance::parse_schemaless`), and a binary `.srl` needs no schema
(`meta::srl`). `meta::typed` declares by hand, over a document's own type
table, the roots of the world family, of scenario regions and of the ped
variation family (`CPedVariationInfo`, `CCreatureMetaData`). The ped family's types are also
declared in code, in the PSC's dialect with the bytes no member names
(`typed::ped_types`), and held member by member, offset, size and signature to the
type tables of the 1,832 staged variation and 9 staged creature RBM documents
(`every_declared_ped_class_is_the_table_the_documents_state`), so a record built
from nothing saves with no snapshot and carries the table every shipped document
does
(`without_a_snapshot_an_array_grows_by_a_copy_and_a_record_built_here_saves`);
the family's definition wins over a snapshot's, which leaves those bytes out. A
save refuses a variation whose `numAvailTex` or `numAvailProps` is not what its
arrays hold
(`a_count_the_arrays_do_not_bear_out_is_refused_before_any_byte_is_written`).
Creature metadata in RBF, whose tags carry no types, opens through an adapter
that reads the tags into the shape the typed members read and writes the members
back into the tags they came from (`typed::ped_rbf`): an untouched save is the
file's bytes
(`every_staged_rbf_creature_opens_typed_and_saves_byte_for_byte`) and a record
built from nothing is an RBF document
(`a_creature_built_from_nothing_writes_the_bytes_a_shipped_one_holds`).
The world family's four base classes are declared in code too (`typed::world_types`: `CMapData`,
`CEntityDef`, `CMapTypes` and `CBaseArchetypeDef`, each with the members its typed record names), so
a map or a type file built from nothing saves with no snapshot, as PSO, since an RBM class carries a
signature word that nothing derives; a record built into a file that arrived is conformed to the
class that file's own table states, and to the declaration where a PSO states none
(`an_entity_built_into_a_file_states_the_members_that_file_states`). The declaration wins over a
snapshot there, so every machine writes the same members. A record copied from another file carries
that file's tree, and a file whose table states no class for it takes it the same way: the last
entity of a map taken out and put back, and the first of a map made from nothing
(`the_last_entity_of_a_map_is_taken_out_and_put_back_as_it_was`,
`a_map_and_a_type_file_made_from_nothing_take_their_first_record_as_a_copy`); a member the class
lacks is refused and never dropped, and an RBM file refuses by its signature
(`a_copy_put_into_an_rbm_file_that_states_no_class_for_it_is_refused_for_its_signature`). The same
holds of a copy into a file that does state its class: a member that class lacks there is refused,
where the writer would drop it and say nothing
(`a_copy_whose_member_this_files_own_class_lacks_is_refused_rather_than_dropped`, and
`every_staged_tree_holds_only_members_its_own_type_table_states` for the rule it rests on, that a
tree as it was read holds no such member), and a copy that holds a record of a class the file lacks,
an extension of an entity, is refused by that class's name
(`a_copy_that_holds_a_record_of_a_class_this_file_lacks_is_refused_by_name_and_never_by_the_schema`).
⚠ A class a table only names, for an array that holds nothing, is no class to lay a record out by
(`a_room_put_into_a_file_whose_table_only_names_its_class_is_refused_by_name`).
`crates/io/or-io/src/meta/typed/interior.rs` types the four records of an interior as members of
`CMloArchetypeDef`: `CMloRoomDef`, `CMloPortalDef`, `CMloEntitySet` and `CMloTimeCycleModifier`, each
with the members the installs state for it
(`each_record_types_the_members_the_installs_state_and_no_other`) and what a file states beyond them
carried (`a_room_renamed_saves_reads_back_and_keeps_the_member_no_record_types`). No class of the
four is declared for a file that states none, so a record is added as a copy of one the file holds.
A save refuses an interior whose room or portal attaches an entity past its list, whose portal names
a room it lacks or whose entity set holds two arrays of different lengths, in the sentence
`or_world::validate::check_interior` gives
(`a_broken_interior_is_refused_at_the_save_with_the_sentence_the_check_gives`). ⚠ A record held by
value keeps the tree it arrived as whatever a text read with no schema called it, `Item` for a room
(`a_text_type_file_read_with_no_schema_saves_its_interior_as_it_came_and_takes_an_edit`).
On type files built by hand, as PSO and as RBM, each of the four records reads to the values built
and an untouched save is the file
(`each_record_of_an_interior_reads_to_the_values_built_and_saves_to_the_same_bytes`); a portal whose
rooms are swapped, an entity moved between entity sets and a room added as a copy each save and read
back equal (`a_portal_whose_rooms_are_swapped_saves_and_reads_back`,
`an_entity_moved_from_one_entity_set_to_another_saves_and_reads_back`,
`a_room_added_as_a_copy_of_one_the_file_holds_saves_and_reads_back`); a portal, an entity set, a
modifier and an entity built here take the members the file's own class states
(`each_kind_of_record_built_here_takes_the_members_the_files_own_class_states`); a room of a text
read with no schema reads back equal after an edit of a member only an interior types
(`a_text_room_edited_in_a_member_only_an_interior_types_reads_back_equal`); and each typed member
reads the slot the engine's class declares, where a schema snapshot is staged
(`every_typed_member_of_an_interiors_records_reads_the_slot_the_engine_declares_for_it`). The install
suite is `every_interior_of_an_install_reads_typed_states_the_members_typed_and_saves_to_its_fixed_point`
in `crates/domain/or-world/tests/naming_every_scenario_point.rs`: it skips without
`OPENRAGE_GAME_ROOT`, and with it set it passes on the Legacy install (541 interiors in 2,763 type
files, 424 of the type files saved to their fixed point, 0 problems) and on the Enhanced one (556
interiors in 2,876, 439 saved, 0 problems), at `8853987c8` with this change. The staged corpus holds
no map or type file, so the census of `meta_census::` judges no world root and that run is what
holds them. It measures nothing of which members an
install states for a parked vehicle, an occluder or the far lights, which
[R70o](../direction/Roadmap.md#r70o-parked-vehicles-zones-lights-and-occluders) opens with. Left in
the interior records: whether a shipped interior with no room states the room class is not measured
(the refusal above is tested on a layout a test took the class out of); a portal that has not four
corners is left to the rule `interior-broken` and is not refused at the save; and the read views of
`crates/domain/or-world/src/map_types.rs` still read the value tree, held to the typed records by
`the_read_views_of_an_interior_read_what_the_typed_records_read`. `or-maps` still builds the first
entity of an empty map and the first archetype of an empty type file and copies neither, and where
the donor was made from nothing the record built is to the byte the copy
(`the_first_record_an_operation_builds_is_to_the_byte_the_one_a_copy_of_another_files_gives`).
`meta::typed::weapon` declares the weapon family, the roots `CWeaponInfoBlob`, `CWeaponComponentInfoBlob` and
`CWeaponModelInfo__InitDataList`, whose files the installs ship as XML text: its types are `typed::weapon_types`, a record keeps
the value tree it was read from so that a member no type names (a `ref` to a named item, a null pointer, a shape that differs from
file to file) arrives in a save as it came, and a save is the canonical XML
(`every_weapon_file_of_the_install_opens_saves_and_arrives_as_it_was`, install-gated). `weapon::reload` is what a build's reload
calls for a text `.meta`.
`gta5::formats::Format::open_declared` reads a declared format as its owned root
where there is one, the one dispatch that a build's reload and `openrage-cli
search --verify` share.
The snapshot's
path names its serialised shape (`SNAPSHOT_FORMAT`), because a tree with
another type model cannot decode it: a change to anything it serialises stops
`the_snapshot_format_names_every_shape` compiling until the shape is listed and
the format raised. `or_io::meta::shipped` holds the types shipped files declare
and the PSC corpus does not, and is additive: it never replaces a type the
snapshot declares, so a file naming nothing in it types exactly as without it.
Its suites are `or-io-verify`'s `meta` binary, and the layouts are the local
[Meta_Binary_Formats_Specification.md](../extra/Meta_Binary_Formats_Specification.md).

`or_io::gta6`, behind the `gta6` feature, is GTA VI's formats as ordinary code, since no source names
their fields: a name says the role a field was measured to play, and nothing there is a declaration
([GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md) owns what is read, saved and refused).

| Module | Holds |
|---|---|
| `container::rpf::rpf9`, `rpf9_scan`, `rpf9_write`, `rpf9_grow` | the archive's clear header and the refusal of its encrypted table; the scan that finds bodies by their bytes, its kinds, families and roles, and the scan kept as bytes; a resource written as a loose file; a section grown or shrunk with every typed pointer moved |
| `gta6::headerless` | the headerless resources by role (drawable, fragment, bound, dictionary, navigation cell and octree, cloth, particle effects list and the small ones), and the writers that put an edit back in place or lay a part out anew |
| `gta6::ttpf`, with its `ttpf_write`, `ttpf_relayout`, `ttpf_slice` and `ttpf_calibrate` files | a texture pack read, saved, a texture given another shape, one slice replaced, and a pack's side of a calibration |
| `gta6::bcpack` | the console's entropy coder over BC blocks: `rans` and `tables` the entropy layer, `bc1` the exact BC1 rebuild, `planes` the BC1, BC3, BC4 and BC5 rebuild through tables, `calibrate` those tables fitted from the user's packs; `Decode` counts the blocks a fitted table drew |
| `gta6::hash`, `gta6::names`, `gta6::texture_names`, `gta6::map_names` | the two data hashes; the vocabulary harvested from bodies, the name each model states of itself and the textures each model binds; the rules a texture's name follows from its drawable's, and the use label of a texture with no name; the rules a map's name follows from another map's |
| `gta6::anim`, `gta6::anim_scene`, `gta6::move_network` | animation resources read and written; anim scenes; a MoVE network's node outline |
| `gta6::map` and its `map_grow`, `map_links`, `map_lights`, `map_lanes` and `map_world` files | map placement and archetypes typed from a body's own class table, entities added and removed, level-of-detail and interior links, lights placed in the world, the lane graph joined, and the census, index and grid over many bodies |
| `gta6::text`, `gta6::text_language` | a text table read and written; the language a table has by the order a change set registers its archive |
| `gta6::audio_data`, `audio_tables`, `audio_links`, `audio_radio`, `gta6::dialogue` | audio game data and the items a table lays out, what they link to, the radio's stations joined to their streams, and dialogue lines joined to their audio and their subtitles |
| `gta6::ui_tree` | an RBF0 document as a strict element tree |
| `gta6::ped_catalogue` | a ped catalogue's asset kits and set kits, typed from the value tree by member name |
| `gta6::mission_tables`, `gta6::script_bodies` | a script body's mission state tables, found by its block table; the script archives' opaque bodies, stated and never read |
| `gta6::save` | the three save containers: a story save as an owned value, a record read, a packed copy refused |
| `gta6::other_bodies` | the bodies a scan names by their head |

⚠ `gta6::save` was written from a description of the layout with no save at hand, so its tests hold
it to that description and to nothing a game wrote. A member a typed reader of `gta6::map` or
`gta6::anim_scene` uses is checked against the body's own class table first, and a body that states
another layout is refused and stays the generic tree.

**or-io-macros** is `#[record]`: one declaration makes the owned struct, its raw
layout, its description and its view, with the layout proved at compile time.

**or-io-verify.** `publish = false`: the checks that read what `or-io` does not
ship. `testimony` is the recorded testimony and its check
([Oracle_Testimony.md](../guides/Oracle_Testimony.md)); `diff::structural` compares two
descriptions by position, never by name; `census` counts instances off their
natural alignment and what a save leaves null. The per-format gates are
`crates/io/or-io-verify/src/gates.rs`, which `declared_gates!` writes as a
format's tests. No other crate depends on this one, so `or-cli` and the app
never build `or-test-support`. The compiled readers the gates once compared
against went with `or-asset` in R66f; the checks that outlived them are the
recorded testimony and the behaviour baselines
([Oracle_Testimony.md](../guides/Oracle_Testimony.md)).

**or-kraken.** Decodes Oodle Kraken, the compressor most of RDR2's larger bodies use, through the
pure-Rust `oozextract` port of ooz, vendored in its `oozextract` directory with one fix. It is a crate of its own so that the port's panics end in it
(`.cargo/config.toml` compiles it with LLVM, which unwinds, where the rest of a development build is
Cranelift, which does not) and so that the port's unstated licence stays out of `or-io`, which
reaches it only through its `oodle` feature. It hands the port one 256 KiB quantum at a time, since
the port reads a stream's first eight literals only once, and refuses by name a chunk the port cannot
read.

**or-media.** Decodes audio and video behind one interface, in OpenRage's own Rust
([recorded](../direction/Project_Context.md#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05)).
A `Source` opens from a movie's bytes (`or_media::open`) or from one stream of a parsed `.awc`
(`or_media::awc::open`), states its `MediaInfo` (codec, channels, sample rate, frame size and rate,
duration), and opens an `AudioStream` of interleaved `i16` or `f32` samples or a `VideoStream` of
RGBA frames, each seekable where its format allows. `Decoders` is the one table from `Codec` to
decoder: PCM16, RAGE ADPCM and MP3 are rows that call the decode `or-io` keeps, a codec with no row
is refused by name (`MediaError::NoDecoder`) while its info still reads, and the first row for a
codec decodes it, so a row an embedder adds with `Decoders::with` never outranks a built-in one.
Bink 1, Bink 2 and Bink audio (DCT and RDFT) are rows too, decoded by the port of FFmpeg's decoders
in `crates/io/or-media/src/bink`, and the port of NihAV's in `crates/io/or-media/src/bink2` with the
in-loop filter and the clamp of a vector that RAD's `bink2w64.dll` shows (`crates/io/or-media/src/bink2/filter.rs`), over
the container in `crates/io/or-media/src/bik.rs`:
`bik::Pictures` yields a movie's planar 4:2:0 pictures, `bik::audio_track` the sound of any one of
its tracks, and a `VideoStream` converts a picture to RGBA. XMA is a row, decoded by the port of
FFmpeg's WMA Pro decoder in `crates/io/or-media/src/xma`. GTA VI's codec 2 is a row, `crates/io/or-media/src/spectra.rs`: an inverse FFT per block, each block's missing Nyquist term restored from the overlaps, and `or_media::spectra::replaced` writes one from a WAV. The app adds no row; a Bink 2 movie's
sound is a Bink audio track and decodes through the Bink audio rows; a `KB2n` movie's DCT track has
a row of its own, `Codec::Bink2Audio`, since its blocks state a band's step in seven bits and a
span's signs after its values. A multi-channel `.awc`
stream's blocks stay `or-io`'s to de-interleave, so the table is only asked whether it decodes each
channel's codec. ⚠ **The crate's licence is the strictest of any file in it**: AGPL-3.0-only
today, for the Bink 2 port of NihAV's Rust, over the LGPL-2.1-or-later Bink 1 and XMA ports from FFmpeg n7.0, and the
Bink 2 filter is derived from a proprietary DLL, which the review before publication weighs (owner, 2026-10-06).
Each ported file opens with a header naming origin, upstream path, revision and licence, and
`LICENSE-third-party.md` lists it; `or-io` stays MIT. It depends on `or-io`'s `awc` feature
alone, and `or-io` never depends on it.

## Title crate

**or-title.** Where the bytes live.

| Module | Answers |
|---|---|
| `game/` | which title and edition this is: profiles, the registry, `Game`, and what its game needs beside the executable to read the `mods` overlay |
| `vfs/` | which bytes a `common:/…` path names; a GTA V mount stops at a pack of the DLC list when given `DlcCutoff::Level`, reports every listed pack with the ones left out (`MountedGame::listed`) and keeps its walk out of the folders that hold or patch them, and takes the map as story or as multiplayer from `MountOptions::mode` |
| `dlc/` | which packs exist, their override order, which of a pack's change sets apply to the map as story and as multiplayer (`mode.rs`), and writing one ([Dlc_Pack_Creation.md](../features/Dlc_Pack_Creation.md)) |
| `gta6_mount.rs` | which maps a GTA VI install registers, from the registration documents a scan finds, with no table to read ([GTA_VI_World_Viewer.md](../features/GTA_VI_World_Viewer.md)) |
| `mods/` | which mods an install holds, as `mods` and `mods_` hold them and in the game folder, what each writes, and which of two mods writing one path the load order resolves to |
| `oiv.rs` | an `.oiv` package: the `assembly.xml` a build writes and reads back, and the stored zip it sits in |
| `install/` | a table fetched from an install: clip regions, the materials palette, the shader archive, the names `install::names` indexes, and the file at a path the explorer writes (`virtual_file`, which reads archives as folders and goes into a nested archive) |
| `archives.rs` | stock archive names and their NG key indices |

⚠ A level cuts a pack's device, its patch layer and the walk, and nothing else:
a path written into a folder of a left-out pack (`update:/dlc_patch/<pack>/…`,
or the pack's archive under `dlcpacks:`) still opens and a glob still lists it.
`--dlc`, the older cutoff by name hash in mount order, still mounts every pack
when its name is unknown. The versions a map change set carries in an overlay
information file are not read, so the highest one is not chosen, and the title
update's own setup is not read, so its change sets have no mode.

Adding a title has [its own section](#adding-a-title).

## Domain crates

**or-addonped.** The addon ped engine, headless: ped models a person streams into the game and the
data that defines them, as content of a project, whose design is
[Project_Model_Design.md](../direction/Project_Model_Design.md#what-the-extension-and-the-second-kind-of-content-build).
`model` is `addonped.json` (each ped with a stable id, the name the game resolves it by and the files
it is made of, each with the place it lands in the resource when that is not where `layout` puts it,
the data file that defines the peds, and the waivers); `rules` is the table of rules and `check` applies
it to what `facts` read off the disk and to the names `metadata` read out of the data file; `plan` hands
all of it to a build as one `Contribution` (the streamed files, the data file with the manifest line that
registers it, each ped's name as a claimed name, the figures, and an item for each ped); `verify` reads a
built resource back against the model; and `adopt` is the recogniser (`AddonPed`) that groups an adopted
folder's `.yft` models that its `peds.meta` names and the files named for them, and `start_module`,
which makes the folder of a new module. The data file is shipped as it was written and read for one thing, the `Name` of each entry, since
no typed root declares the game's structure for a ped's data. It depends on `or-io` and `or-project` and on
no renderer or UI, and `openrage-cli addonped` validates and verifies it.

**or-content.** The kinds of content this build knows, listed once. A kind of content depends on
`or-project`, so something above them lists them: `kinds` is that list (clothing, addon peds, weapons), and the
functions here are `or_project::content`'s with it filled in (`plans`, `plans_given`, `plans_from`,
`contributions`, `recognisers`, `start_module`), which the editor and `openrage-cli` call and so cannot
disagree about. Adding a kind is a `Content` impl in its own crate and one line in `kinds`. `template::apply_template`
adds what a template file states and a project lacks, and `sharing::concerns` lists what a project folder holds that is not
the person's to send: Apply a template… and Check before sharing… of the editor and `openrage-cli project apply-template` and
`check-sharing` use them, so a window and a command line say the same.

**or-depends.** What an asset needs and where a model is placed, one graph
over an install read in two directions: `Catalogue` turns a name into a file,
`Resolver` an archetype into the files a streamer loads, and `PlacementIndex`
reads placement backwards. `rules::RULES` records per format what it states and
what only the install states (`every_classified_format_carries_a_ruling`).
`ModelContents` reads what only a body says: the entries of a `.ydd`, whether
a `.ydr` carries the bound a `physicsDictionary` points at, and a `.ybn`'s
header box, from the front of its file alone (`Vfs::open_head`).
`audio::AudioIndex` answers which `.awc` holds a sound: it crawls the install
rather than mounting it, because Legacy keeps its wave banks in archives the
mount does not hold, and joins each container to the `.rel` items naming its
waves.

`world_index::WorldIndex` is the world of a mounted GTA V install by name and
by position: every map with its extents, parent and flags, every archetype with
its type file, box and level-of-detail distance, each map's bounds as
`InstallTables` and the filename convention give them, and the maps and the
carried bounds at a point, through a tree over the boxes. It reads the mount
through `WorldSource`, a root archive at a time and each file from the archive
that lists it (`MountedWorld`), which is the seam a test counts reads through.
`world_index::refresh` keeps the index in a file keyed by the install's path
and each archive's place, size and modification time, so a start after nothing
changed reads no file and a changed archive is read again alone; a file of
another version or key, or one cut short or altered, is refused and rebuilt
(`the_world_index_of_a_mount`). ⚠ The size and the time are `MountRoot::stamp`,
which `or-title` takes before it opens the archive: asked of the disk when the
index is built, they would file what an old mount read under the archive
written since. `openrage-cli world index` and `world at` run
it. ⚠ Every copy of a map, a type file and a bound that an archive holds is
read, and which copy wins is settled when the archives are put together: a row
kept for the copy the mount loads alone would be missing once the archive that
shadowed it changed. A bound is answered at a position only where a map group, a
map's name or an interior carries it, because an archetype's own `.ybn` states
its box about the model's origin
(`a_point_answers_the_bounds_something_carries_whose_box_holds_it`). An archive
is told from the one before it by its size and its time alone, so on a file
system that keeps times to the second one written twice within a second at one
size reads as unchanged, until `world index --fresh` reads it all again. A
root a level of DLC keeps folders out of (`MountRoot::left_out`) is given no
stamp, because its file is the same at every level: it is read at every build,
and what one level walked is never kept for another
(`a_root_a_level_keeps_folders_out_of_is_read_at_every_build_and_never_kept_for_another_level`).
With
no `--kept` the index is kept under the configuration directory, a file an
install. `maps_in_column`, `bounds_in_column` and `bounds_near_column` answer the
maps and the carried bounds over or under a point at any height, and within a
radius of it, through the same trees and with no scan of every entry
(`a_column_answers_the_maps_and_the_bounds_a_scan_of_every_entry_answers`).
`boxless_bounds_near_column` answers the carried bounds whose file states no
box, or a box no place is in, by the maps that carry them and stream there.

`world_collision::WorldCollision` is the static collision of a mounted install,
read as a place asks for it. `stream` opens the bounds files whose box comes
within a radius of a column, each from the archive that lists it
(`Bound::archive`), builds a `BoundCollider` of each once and places it as the
file states, because a file a map group or a map's name carries is in world
space. It holds them in a `CollisionSet` under a budget of bytes
(`BoundCollider::size_in_bytes`) and drops the file asked for longest ago, never
one the question being answered asked for, so a later question reads a dropped
file again
(`the_budget_drops_the_file_asked_for_longest_ago_and_a_later_question_reads_it_again`).
`ground` answers the ground under a point and `highest_ground` the highest of a
column, each with the files held and the files that gave no collider, with why
(`Unread`); a file that does not read is opened once, and a file of the
question that is already held is never the one dropped for a file read beside it
(`a_file_of_the_column_that_is_held_is_not_dropped_for_the_file_read_beside_it`).
⚠ A file is found by the box its header states. One that states none is asked
wherever a map that carries it streams, so a file that does not read is named
and not passed over
(`a_bound_whose_file_states_no_box_is_asked_wherever_its_map_stands`); a shape
that lies outside the box its file states is still not found by a column only it
covers. `colliders` is the set itself, for a ray, a swept sphere or a box.

⚠ A bound an interior alone carries is named (`Streamed::unplaced`) and not
read: its file states its box about the interior, as an archetype's own `.ybn`
does about its model, and what puts it in the world is the entity's placement,
which nothing here reads. So it is named by the maps that stream over the column
and place the interior (`interior_bounds_near_column`), never by that box.
`place` is the seam: a caller that knows the placement hands the collider in
with its rotation, one scale and translation
(`an_interiors_collision_is_named_and_not_loaded_until_a_caller_places_it`).

`openrage-cli world ground` asks it of one position: with no height the highest
ground of the column, with one the ground from `--rise` metres above it down, no
steeper than `--slope` degrees. It says the height, the normal, the material and
the bounds file, or none, and in both cases the files it looked in, those that
did not read and those not loaded. A material is its `phMaterialMgr::Id` word,
the type in its low byte, and the name the install's own table gives that type.
`--level` and `--mode` mount as `dlc-list` does; an index at a level is kept in a
file of its own, and a mode leaves out no bounds file, because it takes or
leaves a pack's change sets and the index reads every bounds file of every
archive mounted; the note names the change sets the mode did leave out
(`a_level_leaves_out_the_bounds_of_the_packs_after_it_and_a_mode_leaves_out_none`).
The flags are held through the verb itself, in a child process, by
`a_level_a_mode_a_kept_file_and_a_fresh_read_are_taken_from_the_command_line`.
The install suite
(`the_ground_under_a_real_installs_road_nodes_is_within_a_step_of_them`, slow
tier, not yet run on an install) asks the ground at an install's vehicle path
nodes. ⚠ Nothing in a `.ynd` states the scale of a node's stored integers, so
the suite tests one reading of it against four rivals (`NODE_SCALES`), and the
share of nodes it asks to be within a step is a floor, not a measurement.

⚠ A declared `drawableDictionary` outranks a loose file of the same stem, and
`assetName` is then an entry inside that `.ydd`. Read as a filename, 76,898
archetypes disagree; with the precedence right, 158,665 of 158,665 comparable
archetypes agree with their own `assetType` (`what_a_dependency_index_derives`,
a retail Legacy install, at `57c5b9a5`). `assetType` is not the dispatch, since
with no schema snapshot it is a bare ordinal.

**or-editor.** View models a panel shows without knowing what read the file:
`Summary::of(pack, type, image, at)` is a property grid for any described type,
and `Outline::of` with `PackPath` is the generic tree the Outline panel falls
back to. A row may carry an `Edit`, which `src/state/pack_edit.rs` applies;
`or-editor` writes nothing. `Summary::of_shown` takes which fields an owned
type shows and marks the rest internal and read-only, with a count, padding and
a pointer, and which fields its declaration types `Hash32` (`Reach::hash_fields`),
which read as a hash whatever they are called; a row holds the scalar its bytes hold, which an edit starts from
rather than its rounded `value`, an enum row its declared variants, and a row
with no `Edit` the reason. A field whose value counts, sizes or lays out data
elsewhere in the file, which `shapes_data` lists by type and field, draws
read-only too, its reason naming where that data changes. An outline keeps a
null slot at its index (`a_null_slot_keeps_its_index`) and says when it
truncates. `Pointee::Many` and `Pointee::ManyPtr` fold together for display and
must branch in a walker, because one is structs at a stride and the other
addresses to follow.

**or-interchange.** A drawable as a DCC tool sees it: `scene`, `extract`,
`gltf`, `fbx`, `import` and `doctor`, with its refusals and traps in
[Mesh_Interchange.md](../features/Mesh_Interchange.md). `doctor` is the mesh doctor's operations
(`decimate`, `levels`, `mirror`, `rig` and `uv`, and `through_scene`, which reads a drawable's meshes, changes
them and puts them back through `import`), with the guarantees of [the mesh doctor](../features/Mesh_Interchange.md#the-mesh-doctor). Export and import take either
target and read and write each as itself; neither converts through the other.
`fbx` reads a binary FBX of version 7.1 or later through `fbxcel` (MIT OR
Apache-2.0, with `libflate`, `indextree` and `string-interner` beneath it) and
never writes one; both readers say what a file held that a drawable has no
place for in `Scene::dropped`. `import` pairs a file with a drawable by the names of its meshes and materials before it
pairs by position (`ImportNames` states the names a caller knows), and cuts a mesh that overflows a geometry
into several (`ImportReport::splits`).

**or-maps.** The maps engine, headless: the map files a person streams into the game, as content of a project, whose design is [Workbench_Views_Design.md](../direction/Workbench_Views_Design.md#maps). It has the shape of `or-particles`. `model` is `maps.json`, the file of a maps content (each map set with a stable id, a name, the sources it is made of as folders of the module, and the names a person typed for the hashes a map keeps, with the waivers); `layout` says where a source lands in a resource (at `stream/`, the one place a FiveM resource has) and in a singleplayer DLC (inside one nested archive the pack registers as map data, with what of that archive is unmeasured); `facts` looks at each source (what it cooks to, what the map file places and the type file declares through `or-world`'s views of the typed roots `CMapData` and `CMapTypes`, with an interior's rooms and portals read off the document by `or-world`'s read views, which a test holds to the typed records of `or-io`); `rules` is the table `RULES` (twenty-three, each a code, a severity and whether a waiver can set a finding aside) and `check` applies it; `plan` hands a build one `Contribution` (the cooked `.ymap` and `.ytyp` files and the models and textures at `stream/`, the manifest of the maps, the line that declares a FiveM resource a map, the figures and an item for each map set), laid out for the kind of build that takes it (`Shape`); `manifest` is the `_manifest.ymf` of the content's maps, written by `or_world::manifest_build` from the manifests of the game the content keeps in its `manifests` folder (`keep`), with a note for each map nothing can be written for; `verify` reads a built resource back against the model, the manifest of the maps included; `adopt` is the recogniser `MapPack` and its `start_module`; and `import` makes sources of game files (a `.ymap` or `.ytyp` as a metadata source, the rest through the general import), refusing with its reason a file no source form holds. `mapping` is a mapping that comes in as a folder or a zip (`or_project::place`): `analyse` finds its resources and loose stream folders and says, with nothing written, what an adoption makes of each file (`Becomes`: a source of which kind, a manifest of the game's maps, a file kept whole, a file an asset escrow locks, which is told by its first four bytes and handed to no reader, a file refused with the reason, an archive left out), whether a build carries each line of the manifest, and what `rules` says of the map files, type files and models as one set; two files that would be one source folder (a map file and a type file of one name) are told apart there, the second kept whole with the reason; `check` says what stops an adoption before it writes (the root named, the folder it is made in, the length of its paths there); and `adopt`, which asks `check` first, makes a project of one of them in a folder that is new or empty, the map files, type files, models and textures as sources of a maps content and every other file kept whole at its place, removing what it made when it cannot finish. Five modules are the placement: `edit` changes what an entity or an archetype says in its source (it cooks the source, opens the typed root, changes a member, saves the root and writes the canonical XML back through `or_source::metadata::replace`, which refuses an XML that does not compile back to itself; an entity or an archetype that is added is a copy, and `add_new_entity` and `add_new_archetype` add one that is no copy, with the members the file's own class states), `operation` names each of those changes as an `Operation`, data whose JSON states a format name and a schema, with `apply`, which does one to the sources of a pack and returns the operation that puts them back byte for byte, and `apply_all`, which does a list or none of it and writes nothing on a dry run (an inverse carries the record, the LOD links and the extents it replaces, and is the XML as it was when it is not proven to save the file as it was, on the file as edited and, unless it holds by construction, on the file the new XML cooks to as well; the first entity or archetype of a file that holds none is built with what the donor states and is no copy; an entity of an archetype no type file of the pack declares is held by the extents where it stands, which `Applied::notes` says, and `Done::notes` in a session; an entity taken out of or put into a map alone leaves the LOD links of the map files under it on other entities, which the same notes say for each such file unless the list holds its renumbering, and which the rule `lod-children-uncounted` or `parent-link-broken` then reports; an entity at or after one a container LOD of the file names is neither taken out nor put in; a source that leaves the folder of the pack is refused, and a rotation is stored at length one). An operation may state `expects`, the SHA-256 of the XML its source is to hold (`digest_of`), and is refused with `Error::Stale` when the source holds another: every inverse states the digest of what its operation wrote, so an inverse applied after another edit of its source is refused, and a step of the Map screen's Undo is the operation and its inverse with no copy of the XML. `Session` (`crates/domain/or-maps/src/operation/session.rs`) holds the sources of a pack open as their typed roots, so that an edit costs no XML: `Session::apply` changes a root and writes nothing, the boxes of the type files are read once and read again after a type file is edited, and `Session::flush` writes the XML of every source that changed through the same guard, all or none, which is byte for byte what `apply` writes operation by operation; a session dropped with operations unwritten flushes, and can only log a refusal. ⚠ A session that refused an operation since its last flush writes nothing when dropped and logs it, since what it did before the refusal may be part of a list that is sound only whole: `Session::apply_all` does a list or takes back what it did of it, `Session::discard` lets a session go unwritten, and `delete::apply_in` does a planned deletion through a session. What a save or the guard refuses is refused at the flush, which names the first operation it could not write (`Error::Unwritten`). An inverse is given with no save where it holds by construction and is proven at the flush, and is done as `apply` does it where it does not (XML typed by hand, an insertion that claims a child another entity has, a name a type file declares twice, the last record of a class taken out of a file). ⚠ A state the session has not written has no XML to hash, so its inverse states a token that only that session accepts; `Written` gives the digest of what a flush wrote, and an operation that states the digest of an XML is refused while its source holds operations the session has not written. `delete` plans taking entities out of the map files of a pack with their chain of far models (`plan` follows each LOD link, inside a file and, where `flags` bit 3 is set, into the map the file names as parent, and says as a `Deletion` what goes, which is the entities chosen, what stands under them and each parent left with no child; which parents stay with their count of children lowered; which chosen entities are left because another of their archetype stands at their pose and the chain of no other chosen entity takes them; what the sources do not hold of a chain; every file written with what changes in it; the list of operations that does it, `remove_entities`, `set_num_children` and `renumber_for_removed`, whose inverses put every file back and which a session takes like the others (`remove_entities` holds by construction unless a class leaves the file with what it takes, and `insert_entities` is done as `apply` does it); and each model a script line would hide instead. It reads the sources of the pack and no other map, so a map that is not among them keeps its links, a parent that counts children no source holds stays, it skips a source that can be no map file and refuses one that does not open and may be one, and it refuses to take an entity out of a file that carries container LODs or out of a map two files each hold entities of), `scene` is a map set as the boxes its entities place, with `or-world`'s placement rules, and `gizmo` is the manipulator's maths with no renderer: a ray through a pixel, the handles, a drag that moves, turns or scales, the snaps, the pick of a box and the ground under it. Two modules are what a search field and the clipboard take: `place` reads what a person pastes as a position with an optional heading (a call of numbers as Lua and C# write one, a table that names x, y and z with its braces or without, bare numbers, a call or brackets behind what a line of a script assigns them to, with a comma read as a decimal comma only between semicolons), as a hash in hexadecimal, unsigned or signed, or as text, refusing a number no map stores, and spells a hash each way with the name a lookup the caller passes knows; `script_text` writes a position and a list of named objects as Lua, JSON and C# from the templates of `crates/domain/or-maps/src/script_forms.json`, which a file of the same shape extends with no code, with the heading in degrees that `edit`'s pose gives, refusing a number that is not finite and more decimals than a 32-bit number has, and the line that hides a model within a radius of a position from the `hide_model` template of a form, by the hash of its name (the call the built-in forms write has not been run in a game). Two modules are the doors of a set, over `or_world::doors`: `doors` reads the name patterns that say a model is a door from `crates/domain/or-maps/src/door_patterns.json`, a starting guess a person extends, surveys a set into one row a door, reading an archetype two type files declare from the first as the scene and the extents do (what found it, what its box reads as, the other leaf of its double door, which is never held against its arc, how far it opens each way and on what it stops), names each source of the set that is not there or does not cook, and holds the rows to the table `DOOR_RULES`, whose codes `rule_of` knows so that a waiver of the pack names them; `door_text` writes a selection of doors, a double door once and whole, through the templates of `crates/domain/or-maps/src/door_forms.json`, where a form named after another project's door script states that it is unverified and begins what it writes with a comment that says so, and a form that could not is refused. `sample` writes small synthetic map files, type files and manifests for tests, which hold no game content. It depends on `or-io`, `or-world`, `or-project` and `or-source` and on no renderer or UI, and `openrage-cli maps` lists, validates and verifies it, applies a list of operations to it, plans or applies a deletion, keeps a manifest of the game for it and lists its doors.

⚠ An operation is not yet a step of the project's History, and no screen holds a session. The Map screen's Undo keeps the operation and its inverse and applies them one by one with `apply`, so an edit and an Undo of the screen each still cost the XML of their source, and the screen's scan reads the disk after each; a flush writes on the thread that asks for it. An inverse that holds by construction is proven on the file as edited in memory; one that does not is proven again on the file cooked from the XML written, which is the file it is applied to, and is the XML as it was when that fails. The two files differ: a file read again names no class it holds no record of and saves no record of one, so the last entity of a map file or the last archetype of a type file is put back by the XML, in a session as one by one (`the_last_record_of_a_class_taken_out_of_a_file_is_put_back_by_the_xml`, and for several taken out at once `a_map_file_emptied_of_its_entities_gets_them_back_from_the_inverse_of_what_emptied_it`), and a file emptied that way takes its next record built from nothing, which a PSO file lays out and an RBM file, as the samples are, refuses for the signature nothing derives. The first entity or archetype of a file that holds none is built and no copy, so it is saved at the operation in a session too and never held to be sure; each of the five operations of a deletion refuses before its first change, since a session keeps the root an operation was refused on; and what the session, the deletion and an operation over a pack that holds too little were each built without is held through the others in `crates/domain/or-maps/tests/a_session_takes_a_chain_deletion_and_a_pack_that_holds_too_little.rs`. A game map is unmeasured. On a synthetic map of 20,000 entities, whose source is 11.7 MB of XML, one pose edit took 2.4 s one by one, and through a session 0.83 ms a step with one flush of 2.2 s for the drag, in the release profile on an Apple M5 Ultra under a load average of 70 (3.0 s, 1.2 ms and 2.5 s in the dev profile under a load average of 79); the flush is the XML that `or_source::metadata` exports, compiles back and exports again, and the operation with its inverse is 3,108 bytes of JSON where two copies of the XML are 23.4 MB (`one_pose_edit_is_timed_on_synthetic_maps_of_two_thousand_and_twenty_thousand_entities`, 2026-11-25, an ignored timing run by name).

**or-maps, what changed.** `changes` says what a map or a type file changed of its base, by object and not by field: `map_changes` and `entity_changes` compare the entities of two typed roots (added, deleted, each one both hold with what it states differently, the unchanged as a count), `types_changes` the archetypes of two type files by name and member, with the entities of an interior, and `source_changes` cooks a metadata source and the base it keeps (`or_source::metadata::compile_base`) and compares the two. An entity is found by its guid where one entity of each file states it, else by its archetype at the same pose, else by its archetype with the nearest pose; two entities of one file with one archetype and one guid are twins, and a twin that moved is listed as ambiguous with the indexes of the entities it may be. The parent links are compared through the matching, since a deletion renumbers them. What a file states of itself beside them (its flags, its extents, the files it depends on) is compared member by member, and what no typed member names, such as a car generator, is said to differ and is not listed. A link is the entity it names and not its number: an entity taken out of the middle of a map is one deletion though every later index shifted, a parent that counts one child fewer because the child is gone is unchanged, and a parent among twins is the same parent while the link stays among them. A link into the parent map is a position in that map's array, which a chain deletion renumbers: `map_changes_under` compares it by the entity it names where a `ParentMap` gives that map's entities, which `PackSources` does for the sources of a pack, so a chain deletion reads as what it took out and a removal alone in a parent map as each link it left on another entity (`a_link_into_the_parent_map_of_a_pack_is_the_entity_it_names_there_and_not_its_number`); `source_changes`, which reads one source, compares such a link as the number it is. A turn is said as the rotation that places the entity, which a `CEntityDef` stores conjugated and a `CMloInstanceDef` as it is. The nearest match needs each entity to be the nearest of the other with no second as near, and two entities of one archetype at one place whose guids both changed are ambiguous, since the order of the file does not tell them apart. Comparing in the other direction swaps what was added and what was deleted, a map compared with itself reports nothing, and four times the entities cost less than ten times the time (`several_thousand_entities_compare_in_time_that_does_not_grow_with_the_square_of_their_number`). The base of a source is cooked to its canonical form, so a source as it was imported reports nothing. `openrage-cli maps changes` prints it through `PackSources`, a name as its text where the reader or the names of the pack give one and as its hash where neither does; a source that cannot be compared, or is not there, is named with the reason and fails the command, and a model or a texture dictionary of the set is passed over ([R70y](../direction/Roadmap.md#r70y-what-a-project-changed-and-putting-it-back)).

**or-maps, a mapping that comes in.** What holds `or_project::place`, `mapping` and the verb, on files and archives built by hand ([R70j](../direction/Roadmap.md#r70j-a-library-of-props-and-what-comes-in)). **A place to adopt from.** `crates/domain/or-project/src/place.rs` lists a folder or a zip archive and writes nothing: a zip is listed from its directory of entries, a file's bytes are read in memory, and only `Place::stage` writes, into a hidden folder inside the project that goes when the adoption ends. A zip lists, reads and stages as the folder of the same files does (`a_zip_lists_reads_and_stages_as_the_folder_of_the_same_files_does`). A name that begins with a dot is left out once, with everything under it, and a hidden folder of a folder is not entered, so the folder a version control keeps in a cloned resource is neither listed nor counted (`a_hidden_name_is_left_out_once_and_nothing_under_it_is_listed`). **An archive that is not safe to unpack is refused whole, by the entry, with nothing written.** An entry whose path is absolute, climbs out with `..`, names a drive or a device of Windows, in either separator (`an_entry_that_leaves_the_folder_is_refused_by_name_whichever_separator_it_uses`); a link (`an_entry_that_is_a_link_is_refused_by_name`); two entries that differ only by case, refused on every host because a project is shared (`two_entries_that_differ_only_by_case_are_refused_by_both_names`); a path longer than the destination holds (`a_path_longer_than_the_destination_holds_is_refused_before_anything_is_written`), and at the listing a path longer by itself than any folder could hold, which also keeps the listing's cost in line with the archive's size: every folder on the way to a file is kept by its whole path, and a name of thousands of folders cost its length squared (`a_path_no_folder_could_hold_is_refused_at_the_listing_by_name`); an encrypted entry (`an_encrypted_entry_is_refused_by_name`); and an entry that unpacks to more than it states, which ends the unpack and removes the staging folder (`an_entry_larger_than_it_states_ends_the_unpack_and_the_staging_folder_is_removed`). The limits are the rows of `LIMITS`, each with its reason, and the argument of `Place::open`: how many entries, the unpacked size, the ratio of the unpacked size to the size of the archive, the size below which the ratio is not judged, and the length of a path (`an_archive_past_a_limit_is_refused_at_the_first_entry_past_it`, `a_folder_past_a_limit_is_refused_as_an_archive_is`). An archive inside is listed and not opened (`a_nested_archive_is_listed_and_not_opened`). A file of a folder whose name holds a `\`, one name there and a folder and a file on Windows, refuses the folder by name (`a_file_of_a_folder_whose_name_holds_the_other_separator_is_refused_by_name`, on a host that can hold such a name). **What a mapping holds, before anything is written.** `crates/domain/or-maps/src/mapping.rs` finds the resources and the loose stream folders of a place and says of each file what an adoption makes of it: a source of which kind, a manifest of the game's maps the content keeps, a file kept whole, a locked file, a file refused with the reason, an archive left out. It says of each line of the manifest whether a build carries it and who states it, and applies the rules of `or-maps` to the map files, type files and models as one set (`the_map_rules_say_what_a_set_built_to_break_them_lacks_before_it_is_adopted`). A zip is reported as the folder of the same files is and adopted to the same project, every byte equal but the ids OpenRage gives a map set and a source (`a_sound_mapping_in_a_zip_is_reported_and_adopted_as_the_folder_of_the_same_files_is`). A place with no resource and no stream folder says what it found and adopts nothing (`a_place_with_no_resource_and_no_stream_folder_says_what_it_found_and_adopts_nothing`), and an adoption that cannot finish removes what it made (`an_adoption_that_cannot_finish_leaves_the_folder_as_it_found_it`). Two files that would be one source, a map file and a type file of one name or two models of one name in two folders, are told apart in the report: the first becomes the source, and the second is kept whole with the reason and a note that the rules do not read a type file kept so (`two_files_that_would_be_one_source_are_told_apart_before_the_adoption`). `check` is what stops an adoption before it writes, the root it names, the folder it is made in and the length of its paths there, and `adopt` asks it first (`what_stops_an_adoption_is_said_before_anything_is_written`). The JSON document states what the text states, line for entry, on a report that holds a line of every kind (`the_document_of_a_report_states_what_its_lines_state`). **A locked file is kept whole.** It is told by its first four bytes, counted, and handed to no reader (`a_locked_file_is_kept_whole_and_counted_and_no_reader_is_handed_it`). A build ships it byte for byte and names it in what it read back, where it stopped the build before (`a_file_an_asset_escrow_locks_ships_as_it_came_and_is_named_and_not_opened`), and a build for another edition, which has no layout to move a locked file from, ships it the same (`a_file_an_asset_escrow_locks_ships_as_it_came_whatever_the_target_and_is_named`, on a FiveM resource; a story pack and an OIV package call the same two functions and were not run with one). What reads the folder a build wrote agrees with it: the check of a resource names the locked file and opens it no more, and the acceptance sheet counts it under its kind though no reader saw what it holds (`a_built_folder_with_a_locked_file_is_read_as_the_build_read_it`); ⚠ the Build tab does not show `Verification::locked` yet, so only `openrage-cli project build` says a file shipped unopened. **The verb.** `openrage-cli project adopt <mapping.zip|folder> --into <folder>` adopts, `--dry-run` prints the report and writes nothing, and `--json` prints the same report as data (`a_dry_run_prints_the_report_as_text_and_the_same_report_as_json_and_writes_nothing`); the project it makes plans and builds (`a_mapping_is_adopted_into_a_project_that_plans_and_builds_with_its_locked_file_as_it_came`). A dry run that names `--into` or `--root` says what would stop that adoption and exits 1 (`a_dry_run_says_what_would_stop_the_adoption_it_is_asked_about_and_writes_nothing`). A model or a texture is judged by its name until the adoption, where its reader is asked: one it refuses is kept whole, and the build of the project stops at it by name until it is taken out, as the adoption says (`a_model_no_reader_opens_is_kept_whole_and_the_build_stops_at_it_as_the_adoption_said`). What is left: the Library tab, the ghost and the placement by surface; the import report of the app showing this report; a locked file drawn as a box; a place of several resources adopted as one project, which today is adopted one resource at a time with `--root`; a line of a manifest a build does not state, such as a script line, which the report names as not carried and the project file records; a map file and a type file of one name as two sources, which the name of a source folder does not allow; a dry run that asks the reader of a model or a texture, which today makes a source to answer; a count of entries held before the directory of an archive is read, which the zip reader reads whole first; and the unicode forms of one name, which are refused only at the unpack, on a disk that makes them one file (`two_names_the_disk_makes_one_file_are_refused_at_the_unpack_and_never_written_over`), and not before as two cases of one name are. **What the report says of a manifest is what the plan does.** `this_is_a_map` is carried only where a map file becomes a source, a `data_file` line that names files by a pattern (`*` in a name, `**` for any folders) is carried once each file it names is stated, by the maps content for a type file it cooks and by the Files content for a file kept whole, and a word whose text holds a backslash is not carried, since the reader keeps it as written and Lua reads an escape (`what_the_report_says_a_build_carries_of_a_manifest_is_what_the_built_manifest_states`). A file a `server_script` line names is kept beside the project and is no file of it (`KeptBeside`), since a build lists every file it ships outside `stream/` under `files`, which a server sends to every client (`a_script_of_the_server_is_kept_beside_the_project_and_no_build_publishes_it`); no script line is carried, so the scripts of an adopted mapping do not run until their lines are added by hand. `.fxap`, the file an asset escrow writes beside what it locks, is listed by a place though its name begins with a dot, kept whole, shipped at the root of a build and listed nowhere, and a root with locked files and none is a note (`the_file_an_asset_escrow_writes_beside_what_it_locks_ships_with_the_locked_files`). A file kept under a stream folder spelled otherwise is placed under `stream` (`a_stream_folder_spelled_with_a_capital_is_adopted_into_a_project_that_builds`), and a file kept at the path a source cooks to is left out with the reason (`a_second_model_of_a_name_where_the_first_cooks_to_is_left_out_and_the_project_plans`). No install was read: every check is on files and archives built by hand. Links, adoption of a folder in place, the thumbnail cache and the import report exist ([R48](../direction/Roadmap.md#r48-projects)). The ruling of 2026-09-22 keeps the XML a Blender export writes out ([R37](../direction/Roadmap.md#r37-metadata-and-xml-at-parity)), and [Workbench_Plan.md](../direction/Workbench_Plan.md#held-back) holds its import back; an `.oiv` of another tool waits for [R50](../direction/Roadmap.md#r50-package-installation).

**or-maps, roads.** `roads` is the path regions a folder holds as sources, every `<name>.paths.src` directly inside it ([R70p](../direction/Roadmap.md#r70p-roads-scenario-points-and-the-navmesh)). It is in `or-maps` because that crate holds the operations and the rules of the world, depends on `or-source` for the form and on `or-io` for the owned `CPathRegion`, and reads no mount. A node is named by its address, written `region:index` as a region's document writes it (`Address`), a link by its node and its place among that node's links, and a word as the document writes it (`Word`). `roads::operation` is the vocabulary: an `Operation` is data whose JSON states the format `openrage-road-operations` and a schema, `apply` does one to the sources and returns the operation that puts every document back byte for byte, and `apply_all` does a list or none of it and writes nothing on a dry run. The operations state a node's values (`street_name_hash`, `distance_to_target`, `coors_x`, `coors_y`, the height `coors_z` that `as_integer1` holds, and the two words whole, the second without the bits that count the links), state a link's word, add a link with the link back that the far node's region states, take a link out with the link back or alone, add a node after a region's vehicle nodes or after its pedestrian ones as a copy of the node it is linked to, take a node out, and put one back with its links, the links that named it and its junction. An operation edits the regions the sources cook to and writes each back as its document through `or_source::paths::Inputs::rules_of`, so a region is accepted only once its document compiles, reads back through the owned type and states the same document again. ⚠ Taking a node out, and adding a vehicle node before the pedestrian ones, renumbers the later nodes of the region: every link and junction reference of the set that names one is rewritten in the same step, a node another region of the set names by a link or a junction reference is not taken out and the refusal lists what names it, and the step is refused while a node it renumbers links to a region the set does not hold, since a link back from that region could not be rewritten; a node taken out that itself links to such a region is refused for its own link back, which would name a node that is gone (`a_node_that_links_out_of_the_set_is_refused_for_its_link_back_and_a_later_one_for_its_number`). ⚠ An operation states no digest of the documents it applies to, as a map operation's `expects` does: an inverse applied after another edit of the set is not refused and rewires the nodes that took its numbers, and nothing holds an inverse past the call that gave it yet. A node the junction table names is not moved, since nothing samples the ground under it again; taken out, it takes its references and a junction no other reference names with the height samples of that junction, and the later references are renumbered. An inverse is given only once it is proven, read back from its JSON, to export every document as it was, and is otherwise the documents as they were (`rewrite_regions`), which is how a document a person typed is put back. ⚠ Only what the declaration names is named: a link's word is set whole because `CPathNodeLink` names no lane in it, and `disable_node` is refused because `CPathNode` names no state that disables a node. `roads::rules` is the table `RULES` and `check`, which reads what the documents state (`or_source::paths::stated`) and not what they cook to, so a link to a node its region does not hold and a node of more links than its word counts (`most_links`, asked of the declaration) are findings: a document that does not read, a region two sources state, a link to a region the set does not hold (a note) or past the nodes of a region it holds, a link the far node does not return, a link that joins a vehicle node and a pedestrian node, which the region of each tells by its count of vehicle nodes, a node no link reaches, two nodes at one place, and a node above or under the ground, measured against a closure the caller passes in the units the file states its heights in. A rule that was not decided is listed with its reason and never passed over: the two of the ground when no ground is given, and the rows of `UNDECIDED`, which is the lanes of a link. `openrage-cli maps roads` lists a folder and applies a list. What holds it, on sets built by hand: every operation followed by its inverse leaves every source byte for byte as it was (`every_road_operation_followed_by_its_inverse_leaves_every_source_byte_for_byte_as_it_was`, which fails for an operation with no case), a list is applied whole or not at all (`a_list_is_applied_whole_or_not_at_all_and_a_dry_run_writes_nothing`), a node another region names is refused with the links that name it (`removing_a_node_a_link_of_another_region_names_is_refused_and_names_the_link`), one none names is taken out with its neighbour renumbered (`removing_a_node_none_names_renumbers_the_links_of_the_neighbour_so_each_reaches_the_node_it_reached`, `renumbering_is_refused_while_a_region_that_may_name_a_renumbered_node_is_not_in_the_set`), an added node keeps the order and the counts (`adding_a_node_keeps_the_vehicle_nodes_before_the_pedestrian_ones_and_the_counts_the_file_states`, `a_link_added_across_two_regions_is_stated_in_both`), the junction table is kept (`a_node_the_junction_table_names_is_not_moved_and_takes_any_other_value`, `removing_a_node_takes_its_junction_and_leaves_each_other_reference_its_own`), and what cannot be is refused (`a_request_that_states_what_cannot_be_is_refused_for_what_it_states`, `a_node_put_in_by_hand_is_refused_where_it_would_misstate_its_region`, `an_inverse_that_leaves_a_source_as_the_operation_left_it_is_not_given`, `a_document_that_cannot_be_written_leaves_the_one_written_before_it_as_it_was`). An edit is also held to a second reading of the set that knows no index, a node told by where it stands and a junction by its patch (`each_node_taken_out_leaves_every_other_node_its_links_and_its_junction_by_where_they_stand`, `each_link_taken_out_leaves_every_other_link_and_goes_with_the_link_back_or_alone`, `a_node_added_beside_each_node_stands_after_its_kind_and_moves_no_link_and_no_junction`). Each rule fires on a set built to break it and on no other, with the severity its brief gives (`each_rule_fires_on_a_set_built_to_break_it_and_on_no_other`, `each_rule_is_listed_with_the_severity_its_brief_gives_it`), and the listing as JSON lists what its text lists (`the_listing_as_json_lists_what_its_text_lists`). On the staged corpus, with no install: the 259 regions as one set hold 77991 nodes and 162904 links, and the rules find 25 places where two nodes stand, no link that is not returned, none to a node that is not there, none between a vehicle node and a pedestrian node and no node unreached (`the_rules_read_every_staged_region_and_no_file_of_the_game_is_an_error`, 2026-11-25), and a node of one region is taken out with a neighbour written in the same step and put back (`a_node_of_a_staged_region_is_taken_out_with_its_neighbours_renumbered_and_put_back`, 2026-11-25). Left: the scenario points, the navmesh rules, every screen, the ground handed to the two rules that need it ([R70g](../direction/Roadmap.md#r70g-collision-as-a-query)), a region made from nothing, and the suite over an install. ⚠ `roads::operation` and `operation` are two vocabularies of one shape and are not one: an inverse of a road operation states no digest of what it applies to, and its list is a format of its own.

**or-maps, merging two copies.** `merge` merges two copies of one map against the file both were made from, which for a map of the game is the game's file: `merge_maps` takes the base, ours and theirs as typed roots and returns the merged root with a `Report`, `merge_types` does it for the archetypes of a type file, an archetype being the one of its name, `merge_files` reads three files and saves the result, and `merge_source` merges another copy into a metadata source, whose kept base is the base, and writes its XML. What a side changed is what `changes` says: `map_changes_paired` gives, beside the changes, the entity of the changed map that each entity of the base was found to be, and the merge compares two files no other way. A change only one side made is taken, a change both made to one value is one change, and the rest is a `Conflict`, which names the records as each of the three files indexes them and which the caller's policy answers with ours, theirs or stop, for each conflict by itself; a stop makes no file, and the report still lists every conflict with what was answered. The conflicts are one member both changed to different values, an entity one side changed and the other deleted, an entity both changed that is not merged member by member, an entity of one archetype both added at one position (a likely double, kept once only when the two state the same of every member and name no parent and no child of their own files), a group of twins, and a member of the file itself. ⚠ Twins are never paired: a group that either comparison lists as ambiguous is one conflict, the twins of the side taken stand in the places of the base's in the order of their files, and that holds when the other side is the base itself. ⚠ An entity one side gave another archetype is taken whole or not at all, since a guid a deletion freed is the guid the next added entity takes, and the comparison trusts a guid. A child left without a parent because its side deleted the parent is not read as a change of the child, so it keeps its parent where a conflict keeps the parent, and it is no conflict where the other side deleted it. An entity a side added keeps its guid when neither the base nor any other entity of the merged map states it, and takes one above every guid of the three files otherwise; the entities both hold keep the order of the base and the added ones follow in an order that does not depend on which side is called ours. The merged map is made of the base: each added entity follows the last, each record is put in its place by the operation of `operation` that an edit puts one with, and the entities that are gone are taken out by the one operation that takes several out of a map with one renumbering (`remove_entities` of `operation`, the one a chain deletion is made of), which renumbers the links, takes a child off its parent's count and refuses where a container LOD names an entity that would move or go (`a_merge_that_takes_out_an_entity_before_one_a_container_lod_names_is_refused_and_one_after_it_is_made`), so four times the entities take under six times as long (`a_merge_of_four_times_the_entities_takes_nowhere_near_sixteen_times_as_long`); a count of children is what its side states less the children of its own file, plus the children of the merged file, so what a child map adds is kept; and the extents are the union of what the sides state, grown to hold each entity where it stands. The result is saved and judged by the rules that judge a file by itself, `or_world::validate` among them, and an error refuses the merge with the reason, which names the base, ours or theirs where that file is refused by itself; a warning is a finding of the report. ⚠ What no typed member of the file names comes with the root it is in, so the side that changed it gives the merged file its container: a side in another container than the base is refused for it, and where both state the same of it the side in the container of the base is taken. A child of twins names the twin its own side has it under when that side is taken for the twins; a link from the other side to such a twin follows the order of the files, and a note says so. A `Conflict` says what each side did (`Disagreement`: the two values of a member, the members one side changed of what the other deleted, what each changed of a record that is not merged member by member, what a double differs in), `Ref::described` says a record as a person finds it (what it places, where, its guid), and a note names the entity it is about, the parent it named and which side deleted that. When one side states what the base states, the result is the other side as it is, byte for byte, once every conflict was answered with it. `Report::not_compared` says what the merge does not look into: what no typed member names is taken whole from the side that changed it, the flags of a file are one number, and the extents are not merged. ⚠ Three readings wait for the owner, each held by a test as it is built. Twins are a conflict though the other side is the base itself, where taking the side that moved one would guess nothing (`a_twin_a_side_moved_is_a_conflict_though_the_other_side_is_the_base_itself_and_taking_that_side_gives_it_byte_for_byte`). A guid two sides gave two different entities both hold is stated twice in the merged map, which the report lists as a finding and does not refuse (`a_guid_each_side_gave_another_entity_is_stated_twice_in_the_merged_map_and_is_a_finding_and_no_conflict`). And a double whose guid a merge gave anew differs from the other side's by that guid, so merging the two again is a conflict (`entities_both_sides_added_under_one_guid_both_take_a_new_guid_and_merging_a_side_again_meets_a_double`). A root built in memory that was never saved is refused in a merge that changes it, since an operation carries a record as the value tree its file saves it as (`roots_built_here_and_never_saved_are_refused_with_the_reason_and_merge_once_saved`), and an entity that took the guid of a deleted one of its own archetype compares as that entity moved, which no merge can tell from a move, and it takes what the other side changed of the deleted one (`an_entity_given_the_guid_of_one_its_side_deleted_is_read_as_that_one_moved_and_takes_what_the_other_side_changed_of_it`). The members of an interior instance merge one by one (`the_members_of_an_interior_instance_merge_one_by_one`); an interior archetype merges the members of its own type one by one and takes its entities whole from the side that changed them, a conflict when both did (`an_interior_and_a_time_archetype_merge_their_own_members_one_by_one_and_an_interior_takes_its_entities_whole`), and its rooms and its portals come with them, which no test builds. The module is `crates/domain/or-maps/src/merge.rs`, with the parts of an entity in `crates/domain/or-maps/src/merge/entity.rs`, the entities of a map in `crates/domain/or-maps/src/merge/map.rs`, the archetypes of a type file in `crates/domain/or-maps/src/merge/types.rs` and the files in `crates/domain/or-maps/src/merge/files.rs`. `openrage-cli maps merge` runs it over three files ([R70r](../direction/Roadmap.md#r70r-other-resources-of-the-server)).

**or-maps, collision.** `collision` is the collision of an area as a person changes it, over the bounds sources of `or-source` (`crates/domain/or-maps/src/collision.rs`). An `Operation` is data whose JSON states the format `openrage-collision-operations` and a schema: polygons of a soup taken out (`remove_polygons`), given a material by the game's value, which is added to the soup's list when it is not there (`set_material`), or moved by a turn and a shift in the space of the file (`move_polygons`), each naming a bounds source as a folder of the pack, a soup by the address its document gives it and polygons by their place in that soup as the source compiles now. `apply` and `apply_all` do one or a list, all or none, and a dry run writes nothing. The game reads an area's collision from up to three files told apart by name, `<area>`, `hi@<area>` and `ma@<area>` (`VERSION_PREFIXES`, the prefixes `or-depends` reads a map's static bounds by), and an operation on one is done to every bounds source beside it that is a version of the same area, on the polygons with the same corners in the file's space to within `SAME_CORNER`; a version that lacks one refuses the operation and is named, and so is a bound its source cannot build again, with nothing written (`a_version_that_lacks_a_polygon_refuses_the_operation_by_name_and_nothing_is_written`, `a_bound_the_source_cannot_build_again_refuses_the_operation_by_name_and_nothing_is_written`). An operation that states `alone` changes its one source. An inverse is `restore_sources`, the document and the glb of every source the operation changed as they were, the glb as hexadecimal, so it is as large as they are and puts them back byte for byte (`taking_the_proposal_out_changes_the_three_versions_alike_and_the_inverse_restores_every_byte`). A moved polygon gets corners of its own, so a corner it shared with a polygon that stays is left where it was, to the bit while the bound keeps its quantum (`moving_the_buildings_polygons_leaves_the_corners_a_neighbour_shares_where_they_were`); a move through a composite's matrix that is no rotation, one scale and a translation is refused. `propose` hands `or_world::collision::associate` the bounds sources that are folders of the pack, each read where its file states it, and `Proposal::removal` and `Proposal::moving` make the operations it comes to: one for each version and soup a polygon is proposed in, `alone`, on what is proposed in that version, so that a list applied whole or not at all writes the versions together (`versions_that_hold_other_polygons_are_each_changed_by_what_is_proposed_in_them`); a bound that is one shape, which no operation takes out, is a note. `object_box` is the box of an entity of a map source: its archetype's box, stated or declared by a type file of the pack, under its placement. `collision::sample` builds a small bound from triangles for tests, with no game content.

- ⚠ The versions of an area the game ships are not copies of one another, so an operation that asks them to be alike is refused on them and a proposal changes each by itself: of the 60,767 polygons of the staged versions of an area that has more than one, each looked for in each other version of its area, 462 are found, over 10 ordered pairs of versions, all of them between one area's version with no prefix and its `ma@` version (`the_staged_versions_of_an_area_hold_polygons_of_their_own`, 2026-11-25). The plan's sentence that the three versions are kept alike awaits the owner ([R70l](../direction/Roadmap.md#r70l-editing-collision)).
- ⚠ A place in a soup is good until the soup is edited: a soup built again holds its polygons in another order, so a proposal is made again after each operation. An operation a proposal makes states the digest of the source it was read from (`expects`) and is refused once the source on disk is another, or once an earlier operation of the same list has built its soup again (`a_list_that_names_one_soup_twice_by_the_places_of_one_proposal_is_refused_at_the_second`); one written by hand may state none, and then names whatever is at those places.
- ⚠ A maps content gives a bounds source no role (`role_of`), so `maps.json` names none: the proposal reads the `.bounds.src` folders of the content's own folder, the rule `source-unused` warns of each, and a build of the content ships nothing of a changed bound.
- A move that takes a polygon outside the box a composite keeps of its child is refused by the compile, as `or_source::bounds` refuses any child that outgrows it, and so is taking every polygon out of a soup, which leaves a bound over nothing (`a_soup_that_is_the_buildings_alone_is_not_emptied_and_the_refusal_names_it`).
- A source is named as the pack holds it, letter for letter: a disk that does not tell `S` from `s` would open one folder under two names and write it twice (`a_source_named_otherwise_than_the_pack_holds_it_is_not_edited`).
- The staged bounds hold three areas with more than one version, and the 462 polygons found are 231 each way between one area's version with no prefix and its `ma@` version, with none in the other eight pairs, among them an area whose three versions hold 10, 2 and 1 soups (`the_staged_versions_of_an_area_hold_polygons_of_their_own`, 2026-11-25). Whether a polygon picked in one version is ever to reach another is the owner's to rule on.
- ⚠ The move built is the building's: the corner a moved polygon shared with one that stays is left where it was. The move of a tool, in which the shared corners follow, is not built.
- ⚠ An inverse asks nothing of the source it overwrites: `restore_sources` states no digest, so one applied after another edit of the source takes that edit with it, and a source whose document was written and whose glb then could not be is not put back.
- Left: the tool, with its pick, its rectangle and the game's list of materials; the proposal inside the preview of a deletion ([R70k](../direction/Roadmap.md#r70k-removing-what-the-game-placed)) and behind a move in the world ([R70i](../direction/Roadmap.md#r70i-selecting-and-moving-in-the-world)), which nothing calls yet, and its step of Undo; a bounds source as content of a maps module; the import of the game's bounds for an object, which the caller does; and the judgement on game files, where the thresholds are unmeasured, on bounds built by hand and on the staged bounds alone.

**or-particle** is the particle editor's half over the owned `.ypt` records in
`or_io::gta5::particle`: behaviours, domains, keyframes and evolutions as the
editor shows them, and `or_particle::list`, which finds rules by name, adds,
renames and removes them, and lays a list out so each rule is held once before
a save.

**or-particles.** The particles engine, headless: the particle effects a person streams into the game, as content of a project, whose design is [Workbench_Views_Design.md](../direction/Workbench_Views_Design.md#particles). It has the shape of `or-weapons`. `model` is `effects.json`, the file of a particles content (each particle file with a stable id, an optional label, the folder of its source and the waivers); `layout` says where a source lands in a resource (at `stream/`); `facts` looks at each source (what it cooks to, whether its rules read, the effects it states and the rules and textures they name); `rules` is the table `RULES` (thirteen, each a code, a severity and whether a waiver can set a finding aside) and `check` applies it; `plan` hands a build one `Contribution` (the cooked `.ypt` files, the figures and an item for each particle file); `verify` reads a built resource back against the model; `adopt` is the recogniser `ParticlePack` and its `start_module`; and `import` makes sources of game files and of `.ypt` files a person adds, refusing with its reason a file the source form cannot hold. The source form itself is `or_source::particles`. It depends on `or-io`, `or-project` and `or-source` and on no renderer or UI, and `openrage-cli particles` lists, validates and verifies it.

**or-plugin.** The in-app plugin host over a `wasmi` runtime. A guest cannot
take a Rust type across the boundary, so the host speaks the description's vocabulary,
and the host patches and never rebuilds (`the_host_patches_and_never_builds`); a
guest that links `or-io` builds its own file and hands the bytes back, which the
app reads back and saves before it keeps them
([R44f](../direction/Roadmap.md#r44f-payload-and-gpu-access)). A `Session` keeps a guest loaded
and calls it each frame, and `surface` is the vocabulary a guest registers and
describes in: panels, commands, settings, widgets and the overlay it draws
([R45](../direction/Roadmap.md#r45-the-plugin-sdk-and-registration-surface)). `extension` runs a validator or an
emitter over the plan of a build: the plan goes in as JSON, findings and text files come out one call at
a time, and nothing else crosses ([A plugin that extends a build](Plugin_ABI.md#a-plugin-that-extends-a-build)); a validator granted the pack reads the clothing as data and states a finding with a fix. `transaction` is several files of the project written together, all or none (`write_files`; the app applies a kept set as one step of the project's History, `src/state/plugins/transaction.rs`). The ABI has a version number and an older plugin still loads (`manifest`, [Versions](Plugin_ABI.md#versions-an-older-plugin-still-loads)).
The contract is [Plugin_ABI.md](Plugin_ABI.md), the registries
`plugins/README.md`.

**or-project.** The project container, which the editor and `openrage-cli project`
share: the `modules` and `outputs` keys of a `.openrage-proj` (`Container`), the
path forms a project file stores (`store_path`, `resolve_member`), the plan of a
build as data (`plan`, a pure function of the container and what the caller saw
of each file), the paths of a project file that only one machine can use (`check`), the lock that makes a second window on a project read it and not write it (`lock`, on a file in a folder the caller names, beside the preferences and not in the project's folder, since Windows refuses to read a locked file and a zip of the project would stop), the steps of a file kept on disk for a history (`snapshot`: the bytes and label of each step, and what a step carries with it, in the `history` folder of `private_dir`, which is `.openrage/projects/<name of the project file>`; the newest 200, ignored by Git) and the executor (`build`, `verify`, and `build_watched`, which says through `watch` the file it is on and the bytes done and is stopped by a flag); `built` reads a file of a build by its path in the plan whether the build is a folder, a `dlc.rpf` or an `.oiv`, and `built::unpack` hands every file of a built pack or package to a function, the archives inside a pack opened and an archive that does not open handed over as the file it is, with the files an `.oiv` installs outside its pack told apart; what `verify` finds is data, `Verification`, which holds every file reloaded, the metadata read back, the names checked, the estimated streaming memory against the `memory` limit and, on request through `check_reproducible`, whether a second build writes the same bytes. A Files module holds the
project's documents, so a file with no `modules` key reads as one and migrates
when the editor saves it. A build writes a fresh hidden `.new` folder, reads
every file back, byte for byte and through the declared reader where one reads
the extension, swaps it in and keeps the replaced build as `.previous`; any
failure removes the fresh folder and leaves the last build alone. A `.src` folder is
planned as a file the build cooks (`Origin::Cook`, found by `find_sources`), through a cook the
caller hands in, since this crate takes no dependency on `or-source`; the cook comes before any
write; a source of the game's own content that is still the game's file is left out of every kind of output unless the build ships it, and a pack's registrations and a package's manifest are written again from what remains, a package whose pack is left with nothing installs no pack and ships the rest, and a build is refused only when it would add nothing (`left_out`). It deletes and
replaces folders, so it refuses to put one where the project is, inside the
project but for `build/`, or where a file it copies is, and to replace a folder
a hidden `.<resource>.openrage-build` beside it does not say it built. `build_all` builds the
resources of a split output and checks every one of them before it writes the
first; a failure while one is written leaves the ones before it built. `clean` removes a resource, the build it replaced, the record of its items and the marker, and only where that marker says OpenRage built it; `built_resources` finds every resource in a build folder by its marker, which is how a clean reaches one that no output declares now, such as the build of a removed output or a split's `<name>_1`.

`sizes` weighs the files of a plan by the content that wrote each (`Plan::made_by`, so a ranking adds up to the plan's totals, or to what a build wrote once it is given the sizes its cooked files came out at).

`resource` checks any resource folder as a server will read it, whoever made it and with no project. `resource::manifest` reads the lines of a manifest (a key followed by text or by a table of text, in single or double quotes, with comments and a glob), runs nothing and keeps the lines it could not read. `check` reads the manifest, the files it lists and its data-file lines name, and every file under `stream/`, which it opens through the reader a build reloads its own files through (`readers`); a file that opens with the marker of a server's asset escrow is counted, named and never opened, and a map, a type file or a `.ymt` kept as text is refused, since those readers also take the text a source is kept as. The rules are the table `RULES`, each a code, a severity and a sentence, and `Report` is what they found as data with its text lines. What an install knows of a name the folder lacks is asked through the trait `Install`, which the caller implements because this crate sees no mount, and the ceiling for the streaming memory of one file is the `file_memory` entry of a `limits` map, with no number of its own in the code. `openrage-cli project check-resource` runs it; `each_rule_fires_on_the_folder_built_to_break_it_and_on_no_other` holds every rule to a folder built to break it. No manifest costs the reader (`crates/domain/or-project/src/resource/manifest.rs`) its stack (`brackets_opened_to_any_depth_cost_no_stack`), a sound folder reports nothing, a locked file is held by `a_locked_file_is_counted_named_and_not_opened`, and with an install the names a folder lacks are asked of a catalogue built by hand, on no install. Adoption reads a resource folder too (`crates/domain/or-project/src/adopt.rs`). It reads the files and never says a server starts the resource or the game loads a file. Open in it: no file is weighed without the `file_memory` limit, because the size at which a server warns has not been read from a server and is not in the tree; the types a server streams (`STREAMED`) were read from no server either, so a file of another type is a warning; `model-unused` is not run on a folder, where a model may be a vehicle's or a ped's that no type file names; nothing judges the game line, so a resource of another game has its files opened as GTA V's; a pattern of the manifest is not matched against the folder; that a version line or the line that declares a map is absent is said only of a manifest read whole; and a name is found whatever its case where the machine that checks ignores case, which a server on Linux does not.

`server` reads the resources of a server together. `scan` walks a resources folder through its category folders in brackets, takes as a resource each folder with a manifest and keeps a folder that is neither in `Scan::ignored` without walking it; a link to a folder is not followed and is kept in `Scan::unlisted`, so the scan is not whole and the warning `folder-unread` names it, since a server follows one (`a_link_is_followed_by_neither_the_scan_nor_the_configuration`); it reads each manifest through `resource::manifest` and each file under a `stream/` once, and keeps names, sizes and SHA-256 digests and no bytes: a type file or a map is held only while the names it states are read, and a file that opens with the marker of an asset escrow is digested and handed to no reader. Every file is opened through the trait `Files`, which is where a test counts the reads. `findings` gives rows of the table `RULES`, each with the resources and the names it is about: files of one name in two resources (`file-in-two-resources`, or `file-in-two-resources-same` as a note where every copy is the same bytes), an archetype two resources declare, two folders of one resource name (`resource-name-twice`), what was not read of a resource, and, through the trait `GameFiles`, a file of the name of a file of the game, which is a note. A manifest of maps, a `.ymf`, is in none of these: every build that keeps one writes it under one name, and it is kept by the resource that ships it (`two_resources_that_each_keep_a_manifest_of_their_maps_do_not_clash_over_its_one_name`). Names the same resources share are one finding, as mods that write the same paths are one conflict in `or_title::mods`. `server::config` reads a server's configuration as far as its `ensure`, `start` and `exec` lines, the included files each once and where their line stands, so a finding says which of its resources starts last (`Finding::starts_last`); a resource's place is the first line that starts it or a category it is in, the order is said to be not known where a later line starts one of the resources again, a path that leaves the folder the configuration is in is not followed, and the order is said to be not known when no configuration is given or an included file was not read. `Config::read_beside` takes the scan too and reads an `exec` of `@resource/path` inside the folder of the one resource of that name, where `Config::read` keeps such a line as an included file that was not read. `concerning` asks a scan what a build is about to write again, from the plan's files under `stream/` and what `Declares::of` gives, leaving out the resource of the build's own name. `Declares` is the plan's archetype claims (`Plan::claims`, kind `ARCHETYPE_CLAIM`), which only a content of maps states, and the archetypes of every type file the plan ships under `stream/`: one copied or generated is read from its bytes and one cooked through the cook its caller hands it, and one that does not read is kept in `Declares::unread`, which `check-build` prints in place of its all-clear (`a_type_file_a_build_ships_as_a_file_is_asked_about_by_the_archetypes_it_states`). `check-build` also prints the problems of a plan that cannot be built and exits 1 (`a_build_whose_plan_has_a_problem_is_not_told_that_nothing_concerns_it`). `placement` asks it about the folder a build is written as: where that folder is inside the server's, a folder above it that is no category (`build-folder-outside-category`) and another folder of the resource's name (`build-resource-elsewhere`), with nothing said of a build written outside the server's folder or inside a resource of it, which is where a project adopted in place builds (`a_resource_adopted_where_it_stands_is_asked_about_its_type_file_and_is_placed_nowhere`). Nothing calls either but `openrage-cli project check-build`, and `openrage-cli project check-server` prints a `Report`. Open in it: nothing says which copy a server loads; that a folder in brackets is a category, that a streamed file is resolved by its name alone, what the three commands do and that `@resource/path` names a file of that resource were read from no server, and a `stop` or a `restart` line is not read; a type file or a map that does not open is the note `resource-unread` here and the error `file-unreadable` of `check-resource`, which is the owner's to rule on; a type file outside `stream/` is not read; what a server does with two folders of one resource name was read from no server, so the finding names them and no more; a name is compared in lower case for a file and exactly for a resource; and every file is digested, which on a large server reads every byte once. Each rule is held on folders built by hand: `a_category_in_brackets_is_walked_and_a_plain_folder_with_no_manifest_is_no_resource`, `a_locked_file_is_counted_digested_and_handed_to_no_reader`, `two_resources_that_stream_one_name_are_a_warning_for_different_bytes_and_a_note_for_the_same`, `two_resources_that_declare_one_archetype_are_named`, `two_folders_of_one_resource_name_are_named_and_a_line_that_starts_the_name_orders_neither`, `a_folder_that_cannot_be_listed_is_a_warning_and_the_scan_is_not_whole`, `a_streamed_file_that_cannot_be_read_is_noted_and_is_never_said_to_be_the_same_bytes`, `a_configuration_says_which_of_two_starts_last_and_without_one_the_order_is_not_known`, `a_file_of_a_resource_that_a_configuration_includes_is_read_inside_that_resource`, `a_build_is_told_only_what_concerns_the_files_and_the_archetypes_it_writes` and `a_build_written_where_a_server_does_not_look_is_warned_of_and_another_folder_of_its_name_is_named`; a resource whose manifest does not read stays in the result with its note, and a folder under which no resource is found is refused by both commands, since it would report nothing and read as passed. ⚠ `check-server` and `check-build` exit 1 on a warning, and `check-resource` exits 1 only on an error.

`acceptance` is the sheet of what a person looks for in a running game, since no build can say the game loads a file ([In_Game_Acceptance.md](../guides/In_Game_Acceptance.md)). `acceptance.json` beside it holds the lines, each about one kind of file a build writes, which is an extension the build reads back (`readers::reads`, the table `reload` answers from) or the archive a pack is, with the results a person wrote down. `Sheet::parse` refuses a line or a result that is wrong and names the line by its id, `Sheet::about` gives a build the lines about the kinds of file it wrote with the newest result of each for an edition (`a_build_that_wrote_a_map_and_a_type_file_is_told_their_lines_and_that_nobody_checked_them`), `Sheet::seen` reads them as the kinds nobody saw the game load, or the day all were seen (`Seen`, whose `sentence` is the English that `openrage-cli project build` prints and the Build tab says in the language in use), by the results from where the build loads, story mode for a pack or a package and a server for a resource (`Place::of_output`, `Place::of_built`): a run on a server says nothing of a pack's maps (`a_result_counts_for_the_place_it_was_seen_in_and_a_pack_is_not_seen_by_a_run_on_a_server`). `Sheet::silent_sentence` names, beside a reading, the kinds the build wrote that no line is about, and `kinds_in` counts the kinds of a built resource, pack or package and refuses a folder that is none of them, such as a project's or the one it builds into. Nothing in it writes a result. ⚠ A line is matched by the kind of file alone, an extension: the line of a map made from nothing, which is a PSO where the game's own maps are RBM, is read for every build that wrote a map, so maps are not said to be seen until that line has a result too, and a build of none is told of a line it cannot check. The guide's example of a result is one the sheet reads (`the_guide_writes_a_result_the_way_the_sheet_reads_one`).

The Build tab reads the sheet under what a build read back. `Sheet::seen` gives the kinds of file a build wrote that a line is about and that nobody saw the game load, for the edition the build was written for and for each of the two when it states none, or the day of the newest result when every such line was last seen to load (`a_build_is_told_the_kinds_nobody_saw_loading_for_its_edition_or_the_day_all_were_seen`). The Verification part says it in one sentence that names the kinds as a person does and never says that a file loads, with a link that opens the guide at the steps that load a build of that kind (`a_build_that_wrote_a_map_and_a_type_file_names_both_kinds_nobody_saw_loading`, `the_link_opens_the_acceptance_guide_at_the_steps_that_load_the_build`); a result of loaded in a test's own copy of the sheet turns the sentence of that edition alone (`a_result_of_loaded_for_legacy_is_said_with_its_day_and_an_enhanced_build_stays_unchecked`, which also holds that a build already written is read for the edition it was written for when the build's edition is changed after it), a pack is read against the line about a story pack and sent to the steps for story mode (`a_pack_built_by_the_app_is_read_against_the_line_about_a_story_pack_for_its_edition`, `the_link_under_a_story_pack_opens_the_steps_for_story_mode`), a build of files no line is about gains nothing (`a_build_of_files_no_line_is_about_says_nothing_of_the_sheet`), and the sentences wrap in a narrow column in every catalog (`each_sentence_wraps_inside_a_narrow_column_in_every_language`). `openrage-cli project build` prints the same reading under each resource (`the_build_of_the_recipe_says_which_kinds_nobody_saw_loading_as_the_sheet_reads_them`), and so does `openrage-cli wardrobe build`, the other verb that prints what a build read back (`the_build_of_a_clothing_pack_says_that_nobody_saw_a_story_pack_loading_from_either_build_verb`), in the English the tab says (`the_tab_and_the_command_say_a_reading_in_the_same_english`). Help ▸ In-game acceptance guide shows the guide in the window of the Workbench guide, which now draws its table, its commands and its links (`help_opens_the_acceptance_guide_from_the_top_with_its_table_and_its_commands_as_text`), each row of the table whole and inside the page (`the_table_of_lines_is_painted_whole_with_no_row_over_another_and_none_past_the_page`). Every one of these runs on `or_maps::sample`'s files and on no install.

Content reaches a plan through one contract, `Contribution`: the files and
documents, manifest lines, claimed names, named figures and stable-id items
that one module hands over. `plan` is `plan_with` over what the Files module
contributes; `merge` holds the policy (a manifest key that describes the whole
resource must agree, any other adds up, a name claimed twice is refused across
the whole project even when an output splits, and figures add up against the
output's `limits`); `plans` divides an output that declares `split` at content
boundaries; `diff` and `since_last_build` say what moved, changed, was added or
removed since the last build by id, and `Diff::redirect_map` exports the moves.
A build records its items in a hidden `.<resource>.openrage-items.json` beside
the resource.

- A key this build does not know is carried, and a module or output of a type it
  does not know is carried and refused by a plan, so an older build opens a newer
  file (`an_older_build_carries_the_new_keys_and_a_newer_file_opens_in_this_one`).
- ⚠ The editor's session (`workspace`, `open`, `active`) stays in
  `src/state/project.rs`, which reads the container through this crate; a file
  `Project::create` writes opens in the editor as an empty session. The editor writes
  `workspace` as the file stated it and leaves the documents outside the project's folder
  out of the shared file, in `src/state/project/machine.rs`.
- `readers` reloads a written file by its extension: a declared RSC7 format through
  `Format::open_declared`, a DDS, and a `.ymt` as the metadata document it is. The one
  document a plan writes as text is `fxmanifest.lua`, from general knowledge of the
  FiveM format, and a file whose bytes are not text is a `Generated` origin; nothing
  here has run in game, and a build says only that its files read back.
- A clothing or addon ped module is a kind this crate knows and keeps a folder for (`Module::path`), and
  contributes nothing itself: the `Content` trait of `content` is what a kind implements to hand a plan
  its modules' contributions, to be recognised in an adopted folder and to start an empty folder, and
  `content::plans_given` takes the list of kinds from its caller, which is `or-content`. `content` also holds
  what every such kind shares: `facts` looks at the files a module names (a listing for a crowded folder, a
  digest remembered while a file is unchanged, and from `Depth::Measure` on what a measurer a kind hands it
  counted in each file, remembered with the digest and handed from one session to the next as text by
  `remembered_under` and `remember_from`), `rule` is the severity and the row of a rule with its limit where it has one, `check`
  the lines a verifier reports, and `folder` the folder of a module, a new stable id, the lower case
  name the game resolves and the atomic write of a model file.
- `extension` is the plan as the JSON document a script or a plugin reads (`Plan::document`, the one
  `openrage-cli wardrobe plan` prints and [Plan_Document.md](Plan_Document.md) describes, and `Plan::guest_document`, which a plugin is handed without
  where on the disk each file is copied from) and `Plan::with_emitted`, which adds the text files an
  emitter returned and says in the plan's problems what it cannot take.
- `links` is a content whose folder is another project's (`Module::is_link`, the `link` key):
  `add_link` and `remove_link` edit the container, `links` reads where each leads and which
  project holds it (`Link::owner`), `refuse_write` is how anything that edits a pack is told it
  may not write a linked one, and `refuse_member` says why a file has no place in the project.
  `adopt` groups what a folder holds by what a `Recogniser` finds in it (`analyse`, `Adoption`),
  `place` is a place to adopt from, a folder or a zip archive read as one (`Place`): it lists
  the place and writes nothing, leaving a hidden name out once with what is under it (`left`),
  hands over a file's bytes in memory (`head`, `read`), and stages
  the files asked for in a hidden folder inside the project that goes when the `Staged` it returns
  is dropped; an archive is refused whole with a `Refusal` that names the entry (a path that is
  absolute, climbs out, names a drive or a device, a link, an encrypted entry, two names that
  differ only by case, a path longer than `Limits::path` with the folder it is unpacked into or,
  at the listing, by itself, more than the rows of `LIMITS` allow, an entry larger than it
  states), and the limits are an argument whose defaults and reasons are those rows; a build names
  a file an asset escrow locks in `Verification::locked`, equal to what the plan copies or would
  have moved to another edition, and hands it to no reader,
  and `template` is the file another project is made from (`Template`, `.openrage-template`);
  `edit_container` rewrites the container of a project file and carries every other key. All
  three are described in [what links, adoption and templates build](../direction/Project_Model_Design.md#what-links-adoption-and-templates-build).
- `reference` is what a content needs and does not own, a `<name>.ref.json` in its folder in one of three
  forms (`Reference::Here`, `Other`, `ById`) that `resolve` leads to a file, a source folder, an id alone or a sentence that
  says where it looked, and `content::content_owning` says which content owns a source folder, so that the Files content
  does not ship it ([what references build](../direction/Project_Model_Design.md#what-references-build)).
- An output has a kind (`OutputKind`: a FiveM resource, a singleplayer DLC, an OIV package) and
  may declare a `target` (`Target`, Legacy or Enhanced), in which case the plan marks each
  `.ydr`, `.ytd`, `.ydd` and `.yft` as an `Origin::Retarget` and a build moves it through
  `or_io::gta5::retarget::retarget_file`, or copies it when it is there already. `dlc` and
  `oiv` (private) plan, write and read back the other two kinds; `build_with` and
  `build_all_with` take the shader database a moved drawable is respelled through, and
  `Built::losses` names what a file dropped. `build_observed` and `build_all_observed` are the
  same builds telling a caller each `BuildStep` as it is entered, which the app's Build tab and
  job show. The DLC and the package are written by
  `or_title::dlc::build_pack` and `or_title::oiv`, so this crate depends on `or-title`.

**or-source.** The folder an asset is edited as and compiled back from,
`<name>.<kind>.src`, whose design is [Source_Format_Design.md](../direction/Source_Format_Design.md):
`asset` reads and writes `asset.json` and knows a folder for a source by its header
and not its name, `json::canonical` writes every file of one with keys sorted,
two-space indent and LF, `textures` extracts a `.ytd` of either target into a folder
of exact DDS files and a `textures.json` and compiles the folder back (channel
packing, and a re-encode of only the blocks an edited image changed, in
`reencode`), `optimise` plans and writes the format, the mips and the size of the textures of a textures source (a mip chain added under the level that is there, a halving that keeps the levels below the top, a re-encode only where the size or the format changes, and the DDS of a texture that needs nothing left byte for byte), `models` does the same for a `.ydr` or a `.ydd` as a glb, a
`drawable.json` and the base it replaces the geometry of, and imports a glb, a glTF
or an FBX into one (`drawable_values` is what that `drawable.json` states of the base, the shaders and their parameters, distances, bounds and bone tags, which a compile sets where they differ), `fragment` does it for the common drawable of a `.yft` with the rest of the
fragment the base's, `particles` does it for a `.ypt` as one `particles.json` of its rules, the textures of its dictionary as a textures source and its models kept whole as `models.ypt` ([Source_Format_Design.md](../direction/Source_Format_Design.md#what-wave-p43-built)), `bounds` does it for a `.ybn` as a glb of its triangles and a `bounds.json` of what a glb cannot say, laid over the base (`bounds::edit` reads the polygons of the bound such a source compiles to, leaf by leaf in the order a collider numbers them, and states one soup again with other polygons, which the compile then checks: `leaves`, `Inputs::restated`, `crates/domain/or-source/src/bounds/edit.rs`; the 74 soups of the 21 staged `.ybn`, 132,858 polygons, each restated with what it holds cook to the canonical save of their file, and with one polygon taken out the 72 that hold more than one are built again into a bound that validates, none refused: `every_soup_of_a_corpus_ybn_restated_with_what_it_holds_cooks_to_the_canonical_save`, 2026-11-25), and `paths` for a `.ynd` as one `paths.json` of its nodes, links and junctions, with `stated`, which reads what a document states of its nodes and links with none of the checks of a compile, `Inputs::rules_of`, the document that makes the source cook to an owned region, and `write_rules` ([Source_Format_Design.md](../direction/Source_Format_Design.md#bounds-and-path-regions); `document` reads the members of such a sidecar by name, each fault with its place), `metadata` makes a `.ymt`, `.ymap` or `.ytyp` into canonical XML
beside the file it came from and compiles it back through that file's type table
(`or_io::meta::canonical`, `or-io`'s `meta` feature, which `or-source` turns on), and `cook::Cooker` recooks the one asset a saved path belongs to
into a shadow folder, reporting a failure as `Problem` data and leaving the previous
output in place; `textures::replace_image` puts one image into a `.ytd` by the same compile and
`create` starts a source, empty or made of pictures (`new_source`, `author_textures`), and states no platform, `sample` writes the small valid game files a test drops at a door, `game_import::decompile` is the one door a game file's bytes enter a project by, a source folder or an `asset.json` comes in as it is, and `cook::cook_source` is that cook for one folder into a scratch folder, which a
build calls, and `cook::describe` says what a folder will cook to. `game_import` turns a game file, an archive entry's bytes or a folder
into sources under `sources/` (a table of kinds, a staging folder renamed into place, a
numbered folder for a name another origin holds, a report with digests that undo exactly
what it made, and for a game's file the hash of its own first cook in `asset.json`, `cook::cooked_sha256`, which is how a build tells the file is still the game's), and `git` keeps the managed blocks of a project's `.gitignore` and
`.gitattributes` (`git::Commit` is what the owner chose a repository holds, the sources, the build or the sources taken from the game, stated by a `# commit:` line of the block that every rewrite keeps); `openrage-cli project import` calls them, and a taken-back import is kept aside and put back by `game_import::Created::copy_to` and `restore_from`, so that Redo makes it again. `origin_check` compares `origin.sha256` with the file an install reads at `origin.virtual_path`, once and when asked, and says the game's original changed; `models::import` puts a glb, a glTF or an FBX into a model source and `Imported::lines` is what `openrage-cli source import-model` and the app's Replace geometry… print or list. `crop` cuts a textures source to a window of its pictures by copying whole blocks of every mip, and refuses a texture made from pictures ([Source_Format_Design.md](../direction/Source_Format_Design.md#cropping-a-textures-source)). `maps` makes a normal map and a specular map of a diffuse texture as new pictures of the source ([Source_Format_Design.md](../direction/Source_Format_Design.md#maps-made-from-a-diffuse)), `budget` gives each DDS texture a ladder of sizes and formats with the PSNR of each and chooses across sources to fit a memory target ([Source_Format_Design.md](../direction/Source_Format_Design.md#budgeting-a-textures-source)), and `textures::inputs_of` says which file each texture compiles from. `schemas` generates the JSON Schema of `asset.json`, `textures.json`, `drawable.json` and `dictionary.json` from the types that read and write them, with `schemars`, and holds the files of `crates/domain/or-source/schemas/` to them; `or_wardrobe::model::schema` is that of `wardrobe.json`, kept in the same folder. It depends on `or-io` and
`or-interchange` and has no UI and no watcher.

- A source is a folder anyone can edit or clone, so what it states never chooses where a
  build writes or how large a buffer is: the id and the name are each one plain file
  name, and a DDS header's sides and mip count are bounded before a size is computed
  from them.
- A DDS is copied block for block into the texture it makes, and a rule that would change a block is
  refused by name where it is stated and never skipped
  (`a_rule_a_dds_alone_cannot_honour_is_refused_by_name_and_one_it_can_is_taken`). The decode-and-recompress
  import in `or_io::authoring::texture_import` is not on that path. An image made to the
  size, format and mip count of its DDS keeps every block it did not change
  (`an_edit_changes_only_the_blocks_it_covers`), and any other image is encoded in full, as
  the rules in `textures.json` say.
- ⚠ A source drops what a DDS and the declared header words cannot say: bytes after
  a texture's declared mips, and the words an Enhanced texture holds only in its
  hidden `__carry`. A compiled Enhanced texture takes those at their declared defaults.
- Shipped Legacy files state a compressed texture's `mip_stride` per row of texels, a
  quarter of a row of blocks
  (`every_shipped_compressed_texture_states_its_stride_per_row_of_texels`), and
  `GrcTextureFormat::legacy_mip_stride` is that word for `or-source`, `GrcTexture::set_pixels`
  and `GrcTexture::from_png`, and the retargeting between Legacy and Enhanced states and
  reads it (`a_texture_keeps_the_mip_stride_every_shipped_one_states_through_both_targets`).
  ⚠ The app's loose-DDS document still states a row of blocks.
- No shipped Legacy texture states sRGB, so an image compiles in the colour space its
  DDS states, which is linear, unless its rules say `srgb`.
- `or_io::authoring::texture_import` is the one list of image inputs: PNG, JPEG, BMP, TGA, TIFF, a PSD
  (its flattened image, read in `or-io`), Radiance HDR and OpenEXR, which the `authoring` feature turns on
  through the `image` crate. Its `Levels` are the rule an image of more than 8 bits comes down by, which
  `textures.json` states as `clamp`, `tonemap` and `exposure`; floating-point light is refused until one is
  stated ([what a rule may ask](../direction/Source_Format_Design.md#what-a-rule-may-ask)). The header
  words of a texture are named lists in `header` (`processed` and the DX11 info bits, from `or-io`'s
  declarations, and every other bit as `0x…`).
- An image or a DDS no entry names is a texture of its own, so an image left beside
  nothing is compiled as one; a file an entry names as its `source` or a channel of its
  `pack` is that entry's input and no texture.
- BC7 is `or-io`'s `bc7-encode`, which `or-source` turns on in its own manifest, so a
  `textures.json` that asks for BC7 compiles in every build that holds the crate. A
  workspace build holds it, so it links the ISPC kernels, and a build of one crate that
  leaves `or-source` out does not.
- A cook gives each asset id one slot of the shadow: a second source with the same id is
  refused while the first still states it. An id or a name that Windows reads as a device
  (`CON`, `NUL`, `COM1` and the rest, with or without an extension) is refused.
- A model source is edit mode only: the glb replaces the geometry of a base that the folder
  keeps as `base.ydr` or `base.ydd`, so it compiles with no install, and `compile.base` in
  `asset.json` is that file's hash. `drawable.json` also states the base's shader parameters and
  texture names, draw bucket, flags, level distances, masks, bounds and bone tags, and a compile sets
  the ones it states otherwise; a source that states none keeps the base's
  ([Source_Format_Design.md](../direction/Source_Format_Design.md#what-wave-p12-built)).
  An extraction compiles what it wrote before it keeps it
  (`a_source_compiles_back_to_the_same_meshes`), a compile saves through the format's own
  validation and reads the file back, and a recook cooks a model as it cooks a texture.
- ⚠ A ped component's palette indexes a skeleton in another file, which a glTF cannot say, so an
  extraction takes that skeleton (`reference_skeleton`, from the ped's `.yft`) and embeds it in
  the glb, and `drawable.json` lists the bones by name: a glb whose skin lists them in another
  order is bound by name (`a_glb_that_lists_the_bones_in_another_order_is_bound_by_the_bones_names`).
  With no skeleton given, the weights stay the base's (`keep_skinning`), which holds only while
  the vertex count does, and a glb that states a skin is refused, because there are no names to
  bind it by (`a_skin_imported_into_a_source_extracted_without_the_skeleton_is_refused_and_not_bound_by_position`).
- A drawable dictionary is keyed by hash and a drawable's own name is not a key: two entries of a
  shipped `.ydd` state one name, so a folder is named by its drawable where that is one plain
  name no other entry has, and by its code otherwise, and `dictionary.json` maps code to folder.
- `drawable.json` and `dictionary.json` refuse a key this build has no code for, so author
  mode's `shaders` are refused by name and not skipped. Its `base` block is for a person to read
  and no compile reads it.
- A fragment source is a model's in the app (`models::fixed_file` and `models::output_name` answer
  for it) and its own kind in the cook: the glb goes into the common drawable of `base.yft`, and
  a `.yft` is never read as a `.ydr` or a `.ydd`
  (`a_yft_is_refused_by_the_drawable_readers_and_a_drawable_by_the_fragment_reader`).
- A metadata source compiles with an empty schema library through the type table of its base, so
  the bytes do not depend on the snapshot the host installed; only a file whose XML reaches the
  canonical fixed point with no value moved becomes one, and any other is kept by the import as a
  compiled file with the reason
  (`a_sample_of_the_corpus_ymt_reaches_its_canonical_fixed_point_through_its_xml_or_is_refused_with_the_reason`).

**or-wardrobe.** The Wardrobe engine, headless: a freemode clothing pack as data and
everything a build of it needs, whose design is
[Clothing_Tool_Design.md](../direction/Clothing_Tool_Design.md#what-the-engine-builds). `model` is
`wardrobe.json`, the file of a clothing content (garments with a stable id, a ped, a slot, a
pinned position or none, a model, variations, the metadata the engine states under its own
names, expressions, and the waivers); `resolve` reads the ped, slot, position, skin and
variation out of a file name, behind a streaming prefix such as `dlc^` or without one, takes an image
for a texture by its extension, and lists what it could not read; `alloc` places each garment
among the drawables of its slot, and cuts a slot past the cap of one collection into `<collection>_1` and on (the cap is the limit of the rule `slot-cap`, 128 unless the pack says another); `rules` is the table of rules (a code, a severity, whether a
waiver can set a finding aside and, for some, a limit with its unit) and `check` applies it, the pack's
`settings` (the `rules` of `wardrobe.json`: the edition it is built for and, for each rule a reason could
set aside, whether it is on, how serious it is and where its limit stands) and its waivers to what `facts`
read off the disk and `measure` counted in each model and texture dictionary; `cache` keeps what a session
read of a pack in `.openrage/files.json`, so that the next reads only what changed; `metadata` writes each ped's variation `.ymt`, the creature `.ymt` the peds
share and the shop metadata; `plan` hands all of it to a build as one `Contribution` (the
files, copied to the layout the staged packs share, the generated documents, the manifest
lines that register the collections, the collections as claimed names, the figures, and an
item for each placed garment whose name is the one a saved outfit points at); `verify` reads a
built resource back against the pack; `shop` reads the shop metadata a resource brings (the
files its manifest registers as shop apparel data and any file whose root is `ShopPedApparel`),
resolves each entry to the texture of a garment, keeps its id as written and carries the other
files registered with the shop; `import` makes a pack of the files of an existing
resource in place, shop metadata included; and `options` are what a build may change about the files of a pack (read from, and written to, the build's own entry), `dump` is the clothes dump a server's scripts read, `layout::Paths` names the folders and files of a pack in a FiveM resource or in a singleplayer DLC, and `adopt` is the recogniser (`Clothing`) that groups an adopted folder's
clothing into a content and lists what it could not place, and `start_module`, which makes the
folder of a template's clothing content with an empty pack; `dress` answers, with no renderer, which
model and texture each component slot of a ped wears (the garments chosen, and the ped's own drawable 0
in every other slot, named from the ped's variation metadata through `BaseOutfit`), and applies the sets of alternate variations in force (`Sets`: those a garment worn causes, none or one; the drawables they change are `Dressing::driven`) and the first person view (`Choice::first_person`: the garments flagged hidden there left out, the first person alternate worn, the ped's own head not drawn), `heels` names what a
feet drawable states in expression 4, `flags`, `mask`, `hair` and `audio` name the rest of what a garment's metadata does by the engine's words (each held to the staged documents), `behaviour` is the change of any of them with the read-back of what a build wrote, `alternate` makes the sets of alternate variations and the `pedalternatevariations.meta` they write and read, `firstperson` the first person alternates, `variations` is the textures of a garment (add, move, remove, tint, and the places a texture can hold with no file: `reserve` and `resort`, with `replace::texture` to fill one), `follow` leads a name of the pack to a file, a source folder, a reference or an id alone, `intake` is the one door of a pack (a dropped game file or picture becomes a source of it) and `source_facts` looks at a source as the file it cooks to, `search` says what a garment is found by (its place, slot, name, the name the game gives its model, group and tags), `query` reads a line of terms over a pack (`tag:wool polys>5000`, each turned round by a `-`) and keeps the lines a pack names, `bulk` says what an edit to the garments of a query would change and makes it as one `Revision`, `changes` words what differs between two garments or two packs, `review` compares two versions of a pack by the garment's stable id, `credits` reads the `credit` of each source a pack names and writes the credits file of a build, `pack_package` writes a whole pack as one zip of sources and settings and unpacks it, `sizes` ranks the files of a plan by garment and by slot and lists what the garments name, and four modules make an edit recoverable. `backup` keeps the
newest versions of `wardrobe.json` in `.openrage/versions` of the pack's folder through the store the
project's History uses (`Snapshots::keeping`), puts the last version that reads in place of a
`wardrobe.json` that does not, keeping the text that did not read, and puts one garment or one texture
back to what a version held. `redo` is the `Redo` that every Undo of `edit` returns: the pack on both
sides of the change and the files the Undo moved aside to `.openrage/held`, which `redo` returns and
`purge_held` deletes. Nothing is written over, so nothing keeps an original or a copy of a file written over. `fix` is Fix all safe: the repair, textures, order and mask steps with a report for each and the findings that stand (`all_safe`), and the repair of one garment's model (`repair_garment`, over the private `repair`, which rewrites a compiled `.ydd`'s counts from their runs losslessly and refuses `FXAP` by name); `textures` is the optimiser of a pack, which copies a textures source that has something to change and has the garments name the copy (`optimise`, `safe_options`). `replace` puts a `.ydd`, a `.ytd`, an image or a `.drawables.src` folder in the place of a garment's model or of one of its textures, `duplicate` copies garments to a ped with files of their own, `variations` adds, takes out, moves and tints a garment's textures, and `export` writes a garment's files out under the names a build gives them; each but the last returns the `Revision` of `edit`. `reopen` makes a project of a built singleplayer DLC or `.oiv` (`reopen`, and `reopen_watched`, which counts the files of the pack it has read and, once the `Watch` it is given is asked to stop, makes nothing: the garments with everything their metadata states, each model and texture a source, a cloth compiled, and every file it could not place kept under `files/` and named), `gctproject` does the same for the one foreign project format read, grzyClothTool's (a zip with every byte XORed with `0x2B`, holding a resource), and `package` writes a garment as one folder of sources and settings (`garment.json`) and adds it to another pack as an equal garment, one `Revision` (`export`, `import`; a shop id the pack already writes is let go and said, and a reopened file whose staged name another file shares is kept under a folder of its own) `lock` is the system's advisory lock on `.openrage/lock`,
which one window holds to write a pack. `freeze` is `wardrobe.lock.json`, the drawable of every garment as the last build that went through shipped it, which the rule `renumbered` holds a pack to (`keep_for` writes it for the contents a build took); `dummies` are the four choices over the garments that hold a place and ship no model (`take_out`, `replace_on_add`, `fill`, `resort`); and `replace_mode` is the plan's note, with its words, of a ped whose files replace the game's own, which the `rules` name by the game's collection. `mesh` is the mesh doctor over a garment (`examine`, which writes nothing, and `run` with an `Op`: levels of detail, the rig, the weights, a mirror and a crop of the texture coordinates, each a new drawables source that the garment names and one step of Undo; the crop is in `mesh/uvcrop.rs`, the optimiser of the textures embedded in a model in `mesh/embedded.rs` and the completion of a material's constants, `Op::Materials` and `run_many` over a whole pack, in `mesh/materials.rs`). `maps` and `budget` run the engines of `or-source` of the same names over the sources the garments name, each in a copy that the garments name and one step of Undo (`budget::plan` writes nothing), and `slots` binds a folder an external editor saves into to a slot of a texture and puts what is saved there into its source. `wind` reads the wind scale of each cloth of a `.yld` and states another in every cloth of one, and `Garment::wind_scale` is what the plan ships in place of the linked file's own; `shop_edit` edits the shop entry of one texture (the price, the restriction tags and the forced components typed, any other element kept as the XML it is written in); `outfit` takes the streaming pages of the heaviest outfit a pack can make; and `dress` also answers for props, one at each anchor (`Dressing::props`), and for the alternate model a garment is worn with (`Choice::alternates`). It depends on `or-io`, `or-interchange` (the mesh doctor's operations), `or-job` (the progress and the stop of a drop of many files), `or-project` and `or-source` and on
no renderer or UI.

- `openrage-cli wardrobe` validates, prints the plan as JSON, builds, diffs and imports, writes the list of rules (`wardrobe rules`) and the clothes dump of a pack (`wardrobe dump`), runs Fix all safe (`wardrobe fix`) and the texture optimiser (`wardrobe textures`, in `wardrobe_fix.rs`), makes maps, plans and applies a texture budget and binds a watched folder (`wardrobe maps`, `wardrobe budget`, `wardrobe slot`, in `wardrobe_maps.rs`), the mesh doctor (`wardrobe mesh`, in `wardrobe_mesh.rs`), reopens a built pack (`wardrobe reopen`), packages a garment and adds one (`wardrobe package`, `wardrobe add`), selects the garments of a query and edits them with a dry run first (`wardrobe query`, `wardrobe bulk`), reviews two versions, credits a garment, ranks the files of a build and packs and unpacks a whole pack (`wardrobe review`, `wardrobe credit`, `wardrobe sizes`, `wardrobe pack`, `wardrobe unpack`, in `wardrobe_power.rs`) and reads a build back resource by resource (`wardrobe verify`), all in `wardrobe_exchange.rs`, says how many drawables are frozen (`wardrobe freeze`) and fills or sorts the dummies of a slot (`wardrobe dummies`), in `wardrobe_places.rs`, which also freezes what a build shipped, which also holds the four routes of `serve` that change a pack (`wardrobe-add`, `wardrobe-set`, `wardrobe-fix`, `wardrobe-waive`; [CLI_Server_Protocol.md](CLI_Server_Protocol.md#routes-that-change-a-pack)), and
  `openrage-cli project` plans clothing content through `or_content::plans`. The app plans it
  through `or_content::plans_given`, with the contribution of each pack that its scan made once for the builds that state no clothing option, so what the
  Wardrobe shows and what a build writes are one reading (`planned_with_model`); a build of another kind or with an option of its own is planned for itself (`planned_for`).
- `edit` is every change a screen makes, so the screen holds no rule: `add_clothes` adds the garments
  that dropped files state, makes the `.ytd` of each picture named as a texture through the compile of a textures source,
  and copies the files in without replacing any; `add_clothes_watched` is the same with a `Watch` that
  tells how far it has got and stops it with the pack as it was; `undo` takes a drop back and
  removes only what it copied, and only the folders that drop made, `apply` sets what one garment
  states, `take_out` and `put_back` remove a garment whole and return it where it was, `rename_pack`
  names the pack, `remedy` is the fix of a finding
  with the sentence that says what it changes, `waive` records a reason, and `create` makes a clothing
  folder with an empty pack. `revise` is any other change made so that it can be taken back, keeping the pack as it was
  and as the change left it, and `undo_revision` takes it back unless the pack has changed since;
  `locate_texture` and `remove_texture` put right a texture that is not there, `moved` says which garments
  took another place, and a drop leaves out a garment the pack holds, of the same ped and slot with the same model bytes and texture bytes (`Added::already_held`). `link_cloth`, `unlink_cloth` and `undo_cloth` link the `.yld` of a
  garment (its child: `ownsCloth` is written from the link) and take that back, `set_shop_id` gives a
  texture an id of its own, and `add_clothes` reads the shop metadata among what is dropped;
  `shop.durty_hashes` names the shop entries that state no id as Durty Cloth Tool does. `apply_all` makes one edit to a list of garments, all or none, and `reorder` swaps a garment's place with its neighbour's in its slot.
- A clothing content that is a link leads to a folder of another project: `module_folder`
  follows the link, and `module_folder_to_edit` refuses it, because that project is its only
  writer. Every write the Wardrobe workspace makes asks it first (`folder_to_edit` in
  `src/state/workbench/wardrobe.rs`), so a linked pack is refused with a sentence that names its project.
- A pinned garment keeps its place whatever happens around it, and an unpinned one takes
  the lowest place no pin holds, so removing a garment renumbers the unpinned ones after it,
  and the diff of two builds says so by id, with a redirect map
  (`removing_a_garment_before_unpinned_ones_moves_them_and_the_diff_says_where`). A place is a
  number in a slot of a ped; a collection holds as many as the cap, so `Placed::position` and `Placed::part`
  are the drawable and the collection the files and the metadata name, and `Placed::place` is what a pin states.
  A pack that has been built is frozen: a garment that would take another drawable is the error `renumbered`
  until it is pinned back, a dummy holds its place, or a waiver says why. A place
  nobody fills is a drawable with one texture, as the staged packs keep.
- The planner is a pure function of the model and of what the disk said of its files; reading
  those at `Depth::Digest` reads every byte of every file a garment names, so a pack of 3.8 GB
  is read whole to be planned once. `facts` remembers the digest of a file by its size and time
  and reads it again only when either changes, and never trusts a file written in the last two
  seconds, so a scan that runs again while the Wardrobe is on screen finds nothing changed
  without reading a byte (`a_file_not_touched_since_is_not_read_again_and_one_touched_just_now_always_is`). The same holds across sessions: `cache` keeps what a session remembered of a pack in the pack's private folder and the next takes it back before its first look, so a pack of 1,946 garments and 13,333 files that took 9.6 s to read and count the first time, in a release build, took 0.14 s and read no file the next (`the_pack_of_a_server_is_counted_once_and_then_read_by_what_changed`, `a_second_session_reads_only_what_changed_since_the_first_kept`). A folder that holds many of the named files is listed once and their sizes and times are taken from the listing, because on Windows asking for a file opens it, and a scan of a pack of 12,000 files that found nothing changed spent two seconds there (`the_files_of_a_crowded_folder_are_listed_once_and_each_reads_as_if_it_were_asked_for_alone`).
- It is held to packs that exist: 1,772 of the 1,839 staged variation documents come back out of an
  import and a plan with the same drawables, textures, component infos, props and anchors
  (`a_staged_variation_document_survives_being_held_as_places_and_planned_again`), and the
  plan of a server's resource of 13,331 drawables and textures has its files, the metadata of
  both peds, the header and entries of their shop files and its creature file
  (`the_plan_of_a_reference_pack_matches_the_reference`, which takes the resource from
  `OPENRAGE_REFERENCE_PACK` and skips without it).
- ⚠ A drawable's `propMask` is read bit by bit (`mask`), and the game builds the names of a
  component's files from it, so a garment that states none gets the mask its skin implies (17
  for the race skin, 1 for the other) and the plan, the read-back and the rule `mask-skin`
  read it. The staged documents state seven values and no value is shared by every slot, so a
  mask a pack states is kept as stated, and Fix states the skin bit again where it alone
  disagrees with the skin. The flags that say a ped has variations follow nothing a document
  holds, so a pack states both true, as the freemode packs do.
- A garment's model may be a `.drawables.src` folder instead of a game file: `plan` plans it as
  `Origin::Cook`, the preview and `openrage-cli wardrobe build` cook it, and `export` writes the file a
  build makes of it. Hide hair is a preset of the hat's hair scale (`hair::HIDES`, which `dress` reads to take the
  ped's own hair off); not built: hide ears as a named flag (the game states none that this build can
  name), a prop placed at the bone it attaches to, and addon peds.

**or-weapons.** The weapons engine, headless: weapons a person streams into the game and the metadata that defines them, as content of a project, whose design is [Workbench_Views_Design.md](../direction/Workbench_Views_Design.md#weapons). It has the shape of `or-addonped`. `model` is `weapons.json`, the file of a weapons content (each weapon with a stable id, an optional label, the name the game resolves it by, the sources it is made of as folders of the module, and the waivers); `layout` says where a source lands in a resource (a model or a texture at `stream/`, a data source at `DATA_FOLDER`, which is `data/`, registered by that path under the type its root names, `DATA_FILE_TYPES`); `facts` looks at each source (what it cooks to, whether it opens, and the weapons its metadata defines); `metadata` reads what the typed family says a weapon is (`DefinedWeapon`: name, model, audio, slot, group and the attach points with their components); `rules` is the table `RULES` (a code, a severity, whether a waiver can set a finding aside) and `check` applies it; `plan` hands a build one `Contribution` (the cooked files, the data file with the manifest line that registers it, each weapon's name as a claim, the figures and an item for each weapon and component); `verify` reads a built resource back against the model; `adopt` is the recogniser `WeaponPack` and its `start_module`; `import` makes sources of game files (a text `.meta` becomes a metadata source, the one row this content adds to what a project imports) and says which weapons they define; `edit` changes one weapon's metadata through the typed family and returns the text before and after, so one Undo step puts the text before back; and `assemble` is the pure query of which part a weapon wears at which bone and which model each is drawn with. It depends on `or-io` (its `meta` feature), `or-project` and `or-source` and on no renderer or UI, and `openrage-cli weapons` validates and verifies it.

**or-world.** The world editor's model over `.ymap` and `.ytyp` (`MapData`,
`MapTypes`, `Archetype`, `ArchetypeIndex`, `MapGroup`, `WorkingSet` and the
rest), each a view over the reader's `MetaInstance`: it borrows and never
copies, reads fields by name hash, and `write` edits the tree against the
document's own layout. `scenario` is the exception: it names a scenario
region's points over the typed `CScenarioPointRegion`. `streaming` is the
other: the rules of which maps a camera streams in and which nodes of their
level-of-detail forest draw, over plain values, for any title. `collision`
is the third, over `or-io`'s declared bounds: a `BoundCollider` is one bound's
shapes under a tree built once, a `CollisionSet` holds colliders by key with
the transform that places each, and the set answers a ray, a swept sphere, a
box overlap and the ground under a point in world space. `doors` is the fourth,
over the views and the collision: it finds the doors among the entities of a
set of maps by the door extension of the archetype and then by name patterns
its caller hands it, reads a leaf's hinge, width and motion from the
archetype's box, pairs the leaves of a double door, and opens a swinging leaf
through its arc against a `CollisionSet` and the boxes of the other entities.
`collision::associate`
is a judgement over the same set and no query: which pieces of the bounds under
the oriented box of a placed object are the object's own, as data a person edits
(`crates/domain/or-world/src/collision/association.rs`).

- `create` makes a map, an entity, a type file and a base archetype from nothing,
  with no donor and no snapshot, over the typed roots: the file is a PSO, RBM is
  refused for the signature word no tool derives, and a guid, the content flags
  and the extents are derived as they are for a file that arrived
  (`crates/domain/or-world/src/create.rs`). A free guid is one more than the largest an
  entity of the map states (`free_guid`), so the guid of a deleted last entity is
  handed out again, which the comparison of a map with its base reads as that
  entity kept ([R70y](../direction/Roadmap.md#r70y-what-a-project-changed-and-putting-it-back));
  and a root that arrived in another container than the one
  asked for is refused (`CreateError::SavesAs`). ⚠ Only what the family types is
  stated: an entity made so states no `lodLevel`, no `priorityLevel` and no
  extension, and an archetype no `assetType`, which reads as the enum's zero. The
  flags a plain prop of the game carries are not measured, so `flags` is the
  caller's and zero when not given; an interior, a timed archetype, a room, a
  portal and an entity set are not made from nothing. An entity that is no copy
  is refused in an RBM map that holds none, and the XML of such a source compiles
  a first entity over a PSO base only
  (`an_xml_that_adds_the_first_entity_compiles_over_a_pso_base_and_never_over_an_rbm_one`).
- `parentIndex` does not index its own map when bit 3 of `CEntityDef::flags` is
  set, and `write::refuse_if_broken` honours the bit. A map name names a set of
  files, which `MapGroup` merges (`crates/domain/or-world/src/group.rs`).
- `manifest_build` writes a `_manifest.ymf` for the maps a pack ships
  (`crates/domain/or-world/src/manifest_build.rs`). It makes none from
  nothing: the document is typed by the type tables of manifests the game
  ships, and each entry is a copy of the one they state for a map of that
  name, since nothing derives an entry's flags. A map taken from the game keeps
  its entry whole, and a map none of them states gets none and is named. A
  derivation that left an archetype unresolved adds to the stated type files and
  never replaces them, and an entry a later manifest states as an earlier one
  does is written once.
  A kept manifest with a member that does not decode is refused and never read
  short, and the manifest written is read back strictly. Of the 1,798 staged
  manifests a build writes from, 185 state a map: their 15,126 entries with
  flags set none in 13,561 cases, so a flag written as nought for a map nothing
  states would be wrong about one entry in ten of the game's, and they state
  9,704 map names in 16,469 entries, 10,614 of which no earlier manifest states
  (`every_staged_manifest_of_the_game_is_written_back_stating_what_it_states`,
  2026-11-25). ⚠ The type table of a shipped manifest names an enum for those
  flags and the one written names none, as for every PSO document the tree's
  writer lays out; the bits are kept. The lists the derivation does not give
  (world-state groups, texture bindings, a type file's own dependencies, interior
  bounds) are written empty, and the cell of the Builds window is still derived
  from the files as a resource takes them.
- Create and delete are set operations, because a map exists for the game only
  through its `_manifest.ymf`. Deleting an entity renumbers every index above
  it in every file, and a delete takes a parent with its children
  (`delete_subtree`); 9,113 Legacy and 9,079 Enhanced deletes keep every link
  (`executing_a_plan_keeps_every_link`, at `c33ae1c1`). Recomputing
  `contentFlags` only adds bits, since nothing derives bit 9, which marks a
  `_critical_` map.
- A namespaced extension type has two hash spellings, `rage__…` in the file and
  `rage::…` in a snapshot, and a kind must match both: 196,119 shipped
  extensions conform with a snapshot and without
  (`what_every_extension_in_an_install_carries`, at `ccec84bf`).
- A map lists its LOD parents ahead of the entities nothing names, so a suite
  that only deletes leaves moves no index and tests nothing.
- A world-state group is named for the map it switches, and its `Bounds` lists
  collision files, not member maps (`what_every_world_state_group_holds`).
- A prefab is typed by its entities' member set, which Enhanced extends, so a
  catalogue is one title's and a prefab of the other is refused
  (`a_prefab_round_trips_every_map`).
- A door is read from its archetype's box and never from the game's door
  tuning, so a hinge, a motion and an arc are proposals
  (`crates/domain/or-world/src/doors/leaf.rs` states the rule). What the closed
  leaf already touches never blocks it, because a leaf stands in a frame, on a
  floor and inside the box of its building
  (`what_the_closed_leaf_stands_in_does_not_block_it`), and the box that is swept
  is cut back at its faces, its bottom and its hinge by three constants measured
  on no install
  (`the_wall_a_leaf_hangs_in_and_the_floor_its_box_dips_into_do_not_block_it`).
- The doors are found among the entities an interior always places and not
  among those of its entity sets, each saying whether the extension or a name
  found it
  (`a_door_is_found_by_its_extension_or_by_its_name_and_a_model_with_no_name_is_counted`,
  `a_door_an_interior_places_is_found_where_the_instance_puts_it`). A leaf's hinge
  is at the end of the width its model's origin stands by, and a box of no finite
  size once scaled is not read
  (`a_leafs_box_says_where_its_hinge_is_and_whether_it_swings_slides_or_lifts`,
  `a_door_with_no_box_or_a_box_that_is_none_is_not_read`). Two leaves that reach
  toward each other in one plane, across an opening their two reaches fill, are a
  double door, and a leaf is in one pair at most
  (`two_leaves_that_face_each_other_pair_and_two_side_by_side_stay_single`,
  `a_leaf_is_paired_once`). `DoorArc` is the volume a swinging leaf sweeps, and
  `clearance` opens the leaf in steps and answers the first angle at which it is
  met and by what
  (`furniture_in_the_arc_blocks_a_door_at_the_angle_the_geometry_gives_and_an_empty_room_does_not`,
  `a_door_against_a_wall_opens_one_way_and_not_the_other`), the same for a scene
  turned, leant and moved as a whole and for a leaf hinged at the other end of
  its other axis
  (`a_turned_box_blocks_a_door_where_it_stands_and_a_scene_carried_whole_blocks_it_at_the_same_angle`,
  `a_leaf_hinged_at_the_high_end_of_its_y_is_swept_as_one_hinged_at_the_low_end_of_its_x`).
  The arc opens to the angle its caller asks, a quarter turn each way in
  `or-maps`, and the three cuts of the swept box are `ARC_SKIN`, `ARC_SILL` and
  `ARC_HINGE_STRIP`. In `or-maps` the door rules each fire on a map built to
  break them (`each_door_rule_fires_on_a_map_built_to_break_it`), the other leaf
  of a double door is not held against a leaf's arc, since it opens with it
  (`a_leaf_of_a_double_door_is_not_stopped_by_the_other_leaf`), a source that is
  not there or does not cook is named
  (`a_source_that_is_not_there_or_does_not_cook_is_named_and_never_passed_over`),
  every door form writes both leaves of a pair
  (`every_door_form_writes_both_leaves_of_a_pair_to_the_decimals_asked_and_reads_back`),
  and the JSON of `maps doors` lists what its text lists
  (`the_json_lists_the_same_doors_as_the_text_with_their_pairing_and_what_the_rules_say`).
- The association proposes and applies nothing. Its rule is in its module header
  and its six thresholds are named constants, none measured on a game file: a
  piece wholly in the box, grown by `BOX_SLACK`, is proposed; a triangle the box
  cuts is listed with the share of its area inside and proposed from `MOST` up,
  which is above the three quarters one triangle of a rectangle cut across its
  middle has; a cut shape that is no triangle is left; ground is left unless
  `AssociationOptions::take_ground` asks for it; and an object lower than
  `JUDGED_FROM` is not judged. Ground is a triangle within `GROUND_SLOPE` of
  level, whichever way it is wound, whose part in the box is no more than
  `GROUND_STEP` above the foot, the lowest point a standing piece reaches in the
  box, so a roof is not ground and the floor the walls stand on is
  (`a_box_building_on_a_flat_floor_gives_its_walls_and_its_roof_and_keeps_the_floor`,
  `a_floor_of_the_buildings_own_goes_with_it_once_its_walls_reach_under_it_by_more_than_a_step`);
  or one whose surface goes on past the box, sharing an edge, through level
  triangles the box reaches and corners within `SHARED_CORNER`, with one the box
  cuts and holds less than `MOST` of, so the ground on the high side of a
  building on a slope stays and so does a deck that crosses the box
  (`the_ground_uphill_of_a_building_on_a_slope_is_left_with_the_ground_downhill`,
  `a_level_surface_that_goes_on_past_the_box_is_left_and_one_that_ends_in_it_goes`).
  ⚠ A flat roof that goes on at one height over the house next door is left as
  ground (`a_roof_is_the_objects_at_a_shared_corner_and_ground_along_a_shared_edge`),
  the floor over a cellar the box holds is proposed, and a wall or a rail of
  something else that the box holds whole is proposed.
- A collision shape is exact, where the particle preview's `BvhCollider` makes
  triangles of the same bound, so the two differ on a curved one. A box
  primitive's four vertices are corners no edge joins
  (`a_box_primitive_names_four_corners_no_edge_joins`), a placing transform is
  a rotation, one scale and a translation or it is refused, a ray in a
  triangle's plane meets nothing, and a query that starts inside a solid meets
  it at distance 0, which the ground under a point passes through
  (`crates/domain/or-world/src/collision/shape.rs`). A shape is refused by asking each of its
  numbers, never its box alone, because `min` and `max` pass over a NaN
  (`a_bound_of_sizes_and_places_that_are_no_numbers_is_refused`). `BvhCollider`
  also leaves out the primitives of a geometry and passes over a polygon that
  names a missing vertex, where the query refuses the bound, and two of its
  suites count its triangles (`a_shipped_disc_bound_collides_with_thickness`),
  so making it a caller changes what the particle preview collides with. The
  staged capsule, disc, cylinder and box bounds each fill the box their header
  states, which holds the axis and the radius each is read with
  (`every_staged_primitive_bound_fills_the_box_it_states`); no sphere bound is
  staged, so a sphere's reading is held by bounds built by hand alone. Of the
  staged corpus, 948 bounds become colliders with none refused
  (`every_staged_bound_becomes_a_collider_whose_ray_and_swept_sphere_agree`,
  2026-11-25). `BoundCollider::size_in_bytes` is what a collider holds in memory,
  which is what `or-depends` budgets by.
- An archetype's bounds match its drawable's only about half the time, because
  the archetype's frame is not recorded in the drawable, and `lodDist` and
  `textureDictionary` are authoring decisions
  (`what_an_archetype_takes_from_its_drawable`). The measured figures behind
  these are [R32](../direction/Roadmap.md#r32-world-data-ymap-ytyp-and-interiors)'s.

## Runtime crates

**or-anim.** Skeletal animation from a decoded `.ycd` to a moving figure,
GPU-free so CI tests it; `resolve` answers which skeleton a document offers.
Skinning is `world * inverse_bind`, in that order
(`reversed_multiplication_order_would_be_wrong`). It stays apart from
`or-simulation`, because a clip is a pure function of time and particle state
is integrated ([Animation_Preview.md](../features/Animation_Preview.md)). Its `gta6` feature poses a
GTA VI animation on a GTA VI skeleton (`crates/runtime/or-anim/src/gta6.rs`): a track moves the bone
whose name's hash is its id, and a track that poses nothing is reported by name.

**or-simulation.** The particle simulation behind the `.ypt` preview, following
the engine's solver and behaviours at full fidelity (D9)
([Particle_Preview.md](../features/Particle_Preview.md)).

**or-renderer.** The wgpu viewport on `RenderContext` and `Renderer`. It has no
egui dependency, which is why the texture preview cache in
`src/state/texture_cache.rs` stays in the app. The particle shader contract is
[Particle_Shader_ABI.md](Particle_Shader_ABI.md). `game_shaders` builds the game's
own shaders into pipelines: it reads a SPIR-V pack into `naga` modules, reads what
each module binds from the module, links the stages of a pass for interpolation,
builds the pipeline with the bind group layouts those bindings ask for, and
draws a triangle with stand-in resources, as the `draw_pass_check` example does
for a whole pack, or a quad with real constants written by name (`constants`, with the
engine's own values in `engine`) as `quad` does, for the `first_render` example and
for every pass of a pack in `render_pass_check`. A `frame` holds a colour and a depth target that
several passes draw into one after another, each testing depth when its pipeline was
built to, and `mesh` draws a mesh's surfaces with the pass its material's effect names, the
material's values and its textures, prepared once as a `MeshScene` that a frame updates and
records into a pass the caller owns, for the colour and depth `Target` the caller says (its `Route` picks the
techniques: the forward ones, the ones that write the G-buffer, or the forward ones for what blends), `screen`
draws any pass over the whole target by a quad with the textures it is given, and `deferred` uses both to
light a G-buffer with the game's `directional` pass, reflecting the `sky` of the day, after `shadow` has multiplied the sun's shadow into the G-buffer and `ambient` has darkened the
ambient scales of the G-buffer by the game's `ssao` passes, `dome` draws the sky of the day with its clouds, and the discs of the sun and the moon on it, where nothing was drawn, `point_lights` and `spot_lights` add the light of the particles as spheres of the game's `point` pass and cones of its `spot` pass, both through `light_volumes`, `fog_volumes` lays their fog over the lit frame as spheres of the game's `vfx_fogvolume` pass, `bloom` blurs the bright part of the lit frame and adds it back, and `antialias` draws the mapped frame to the screen, and `sprite` draws
a particle's sprites with a pass of `ptfx_sprite` (a `SpritePass` for each kind of batch, read
from a second, per-instance vertex buffer), which `particle_sprites` lays a frame's batches out for;
`trail` and `model` do the same for a trail's
strip and a model's indexed mesh, which `particle_trails` and `particle_meshes` lay out, and
`particle_game` bundles the three hosts for a kind of target. `or-shader-tool` depends on it under its
`native-front` and `device-check` features, so the check builds a pass the way the
renderer will. `gpu_timer` writes a timestamp between the stages of a frame on a device that has
timestamp queries (`gpu_timer::FEATURES`) and reads them back a frame late, so that the viewport
reports what each stage took on the device without waiting for it; the Statistics panel and
`or-shader-tool render-drawable --frames` show it.

`instance` holds what every wgpu instance of the app and the tools is given.
OpenRage draws through Vulkan, Direct3D 12 or Metal. **OpenGL is not supported**:
it indexes no table of textures at a computed place and updates no texture in a
step, which a game's shading passes do, so the app opens no OpenGL adapter and
`WGPU_BACKEND=gl` is refused in those words (`backends_from_env`) by the
self-check and by the shader tool's device checks. A shader for Direct3D 12 is
compiled by DXC, since FXC refuses valid ones (a gradient inside a loop, a barrier in a
branch) and has no 64-bit integers: `instance::dx12_compiler` takes the
`dxcompiler.dll` beside the program, else the first on `PATH`, else the newest
Windows SDK's, and falls back to FXC with a warning that says so where there is
none. `WGPU_DX12_COMPILER` overrides the choice. A compiler older than 1.8.2502,
such as an SDK's, loads `dxil.dll` by name and Direct3D 12 opens no device on it
without one, so the folder of a compiler found with its `dxil.dll` is put on the
process's `PATH`. On Direct3D 12 the feature `TEXTURE_INT64_ATOMIC` is offered
and the layout of such a texture refused; `updates_64_bit_textures` asks the
adapter instead.

## Tool crates

**or-cli** holds the `openrage-cli` subcommands (`openrage-cli help` lists
them), built into a binary by the app package; the `serve` protocol is
[CLI_Server_Protocol.md](CLI_Server_Protocol.md). An install census such as
`formats --all` stops classifying at a byte budget, its only memory ceiling,
which `--no-byte-budget` or `--budget-gib unlimited` lifts. `awc which` and
`awc index` read `or-depends`' audio index; `awc export` decodes through
`or-media`, and it and `awc replace` take a multi-channel container as one
interleaved WAV; a GTA VI Opus stream `awc export` writes as Ogg Opus, one file per
stream or channel, or decoded to WAV with `--wav`, and `awc decode-check` decodes every
Opus stream of a container in memory, writing nothing. `convert retarget`, the app's Convert and a project build with a
target share one call, `or_io::gta5::retarget::retarget_file`, which `or-cli` re-exports.
`addonped validate` and `verify` are the rules and the verifier of `or-addonped`; the plan and the build of its content are `project`.
`weapons validate` and `verify` are the rules and the verifier of `or-weapons`, and its plan and build are `project` as well.
`maps changes` prints what each map file and type file of a maps content changed of the base its source keeps, by object (`or_maps::changes`), with what the file states of itself differently on a line of its own, and `--json` gives the same changes as data; it exits 1 when a source cannot be compared, and it is in `crates/tools/or-cli/src/maps_changes.rs`.
`maps merge --base B --ours O --theirs T --out FILE [--take ours|theirs] [--json]` merges two copies of one map file or type file against the file both were made from (`or_maps::merge`), the base's extension saying which kind the three are, and prints what each side changed, what was applied, each conflict with what each side did, what was taken and what that leaves, and under it a line for each record it names as its file indexes it (what it places, where, its guid), the notes, and what the merge does not look into; with no `--take` a conflict stops it, nothing is written and it exits 1, and a merge that cannot be made valid fails with the reason. It is in `crates/tools/or-cli/src/maps_merge.rs`.
`particles list`, `validate` and `verify` are those of `or-particles`, and its plan and build are `project` as well.
`maps list`, `validate` and `verify` are those of `or-maps`, and its plan and build are `project` as well; `maps apply` does a list of `or_maps::operation` operations to the sources of a maps content, all or none, prints the notes of each and remembers in the content the names the list states (`Operation::names`), which a file keeps the hash of alone, and `--dry-run` says what it would change; an operation of the list that states `expects` is refused, and the list with it, when its source does not hold the XML of that digest; `maps delete` takes entities named as `<source>:<entity>`, prints the plan `or_maps::delete` makes of them (what goes, what stays, what is left as ambiguous, each file with what changes in it, and the script line that hides each model, in the form `--script` names within `--radius` metres) and writes it, every file or none, only with `--apply`; `maps keep-manifest` keeps a manifest of the game in a maps content, from the files named or from an install (`--game`), for a build to write the manifest of its own maps from; `maps doors` lists the doors of each map set with what found each, what its box reads as, its pairing, how far it opens each way against the boxes of the other entities and what the door rules say, `--names <file>` gives it the names of models a name a line, for a pack that remembers none, and `--json` prints the same doors as JSON; no verb writes a door form or reads a person's own patterns file yet. `maps roads <folder>` lists the path regions the folder holds as sources, each with the region it states and its nodes, links and junctions, and what the rules of `or_maps::roads::rules` find over the set, with the rules that were not decided and why; `--json` gives the same as data, and it exits 1 when a finding is an error. With `--apply <operations.json>` it does a list of `or_maps::roads::operation` operations to those sources, all or none, and `--dry-run` says what it would change. It is in `crates/tools/or-cli/src/maps_roads.rs`, and it takes a folder and not a project because a path region taken from the game is a source of the project's `sources` folder and of no maps content.
`maps collision` prints what `or_maps::collision` proposes as the collision of an object, an entity of a map source named `--object <source>:<index>`, over the bounds sources the content holds: for each source and each soup the polygons proposed, those left and why, and the operations the proposal comes to. `--box` states the archetype's box where no type file of the content declares it and `--take-ground` proposes the ground too. It writes nothing unless asked: `--apply` takes the proposal out, `--move` and `--turn` move it about where the object stands, `--operations` applies a list a person edited, `--inverse` writes the list that puts every source back, before a source is written, `--dry-run` writes nothing and `--json` gives the proposal and its operations as data. It is in `crates/tools/or-cli/src/maps_collision.rs`, and it moves no entity.
`project adopt <mapping.zip|folder> --into <folder>` adopts a mapping that comes in (`or_maps::mapping`), `--dry-run` prints what it holds and writes nothing, and with `--into` or `--root` what would stop that adoption, `--json` prints the same report as data, and `--limits` states the bounds of `or_project::place`; it is in `crates/tools/or-cli/src/project_mapping.rs`, and `project adopt <folder>` with none of those flags makes a project of the folder in place, as it did.
`project acceptance` prints, for a folder a build wrote, the lines of `or-project`'s acceptance sheet about its files and which of them nobody has checked in game, and `project build` and `wardrobe build` say under each resource they wrote which kinds of file nobody saw the game load, as the sheet holds it (`project_acceptance.rs`).
The GTA VI commands are the `gta6_*.rs` files of `crates/tools/or-cli/src/`, one concern a file, reached from
`crates/tools/or-cli/src/rpf.rs` as the `rpf scan-*` verbs, which address a body by the offset a scan found it at;
`rpf list` and `rpf extract` take an RPF9 archive too and answer from the scan. `ttpf` is
`crates/tools/or-cli/src/ttpf.rs` (a pack listed, exported, replaced into, planned and calibrated), `gta6-save`
and `gta6-mission` are `crates/tools/or-cli/src/gta6_save.rs` and `crates/tools/or-cli/src/gta6_mission.rs`, and
`navmesh scan`, `disable` and `delete` take a scanned or loose cell. A command that writes writes a loose
resource or a new pack, never the archive. `crates/tools/or-cli/src/gta6_translate.rs` runs a GTA VI program
through the front end its payload takes, and the app's Shaded calls the same function
([the commands in one table](../features/GTA_VI_Xbox_Build.md#commands)).
**or-shader-tool** translates shaders, `.fxc` to a pack of SPIR-V modules, with a
binary of its own. Its `inventory` command counts what an install's shaders are
(the bytecode, shader model, stage, container parts and feature flags of every
DXBC or DXIL container, and the capabilities of any SPIR-V module) from the
bytes alone, so a translator is chosen by measurement; it reads a file, a folder
of loose files or an install, and writes JSON with `--json`. Its `spv-check`
command runs translated SPIR-V through the legalizer (`legalize`), naga's reader
and validator and its MSL writer, and counts how far each module gets. With its `native-front` feature,
`translate-check` runs a whole install through `or-shader-front`, the legalizer and naga
in one command, on every core; with its `device-check` feature, `--device` builds a
pipeline from each module that survives on the machine's own GPU, which runs the platform's
shader compiler. `pass-check` then asks the question a viewer asks: it reads each
effect, translates and builds the programs a pass binds (`passes`, `pass_check`),
and counts a pass as built only when every stage it names is, by stage set and by
technique, and with `--device` builds each pass of a vertex stage and at most a pixel stage as one pipeline and classifies how the pixel inputs meet the vertex outputs (`signature`). `translate-check` also counts each translated module's bindings against its
container's reflection (`bindings`). `bake-pack` (`spirv_pack`) writes the SPIR-V
pack: one legalized module per program a pass runs and a manifest of what each
binds and which pass runs which (types in `or-io`'s `spirv_pack`). With its
`render-drawable` feature, `render-drawable` draws one drawable of a file or an install
with a pack into a PNG, taking the scene from `or-cli`'s `export_scene` and the textures
from the `.ytd` beside it, and `render-census` counts how much of an install's
drawables a pack draws (`render_drawable`). `pack_protocol` is what `bake-pack --progress`
says on stdout, one JSON event a line, that the app's `src/state/game_pack.rs` reads.

**or-shader-front** turns a DXBC container into a SPIR-V module with the vendored
dxbc-spirv translator (`crates/tools/or-shader-front/vendor/dxbc-spirv`, MIT),
one call per shader through a thin C shim, with the options naga needs set in
the shim. The C++ is compiled by the crate's build script, with the `cc` crate and
no CMake or Meson, only under its `native` feature, which is off by default: a
build without it has no C++ and every call says so. `BindingLayout` (always
compiled) says where each register binds, and the shim places it. `or-shader-tool`
turns `native` on behind its `native-front` feature, so the editor's own build holds
no C++.

Three modules of it are Rust and always compiled, and write naga IR. `dxil` reads a
container that holds bitcode. `rdna` reads GTA VI's Xbox programs, which ship as native
GPU code: `decode` the instruction words (its opcode numbers are `opcodes`, generated
from Mesa's table, MIT, and listed in `LICENSE-third-party.md`), `cfg` the control-flow
graph, `sym` where each scalar register's value came from, and `to_naga` the lowering of
a pixel, vertex, mesh or compute program, which refuses by name (`NotTranslated`) what it
does not model. `rdna_bind` joins each resource of a translated module to the binding
record its package lists and to the name the effect container's table gives the register,
and refuses by name (`Unbound`) a program the renderer has nothing to give. Every test of
`rdna` assembles its input from the encodings' field layouts and none reads a game file.
For these the crate depends on `or-io`'s `gta6` feature, and `or-cli` depends on the crate.

**xtask** is the repository's own tasks, run as `cargo xtask <task>` (an alias
in `.cargo/config.toml`): the chunked formatter, the encoding and provenance
lints, the pre-commit hook and its installer, the test tiers' environment and
skip ledger, the baseline recorder, the format scaffold, `lane-new`, the janitor and `dist`, the
packager. It
replaces the PowerShell scripts so one implementation runs on every OS, depends
on no workspace crate because the hook runs its binary, and is depended on by
nothing.

## The resource machinery

What a decompressed resource is read and laid out through, under the owned
values and the canonical writer.

- `DatResource` (`crates/io/or-io/src/container/paging/resource.rs`) is a
  decompressed payload with pointer resolution, read as bytes; `PgRscBuilder`
  lays one out.
- System memory is addressed from `0x50000000` and graphics memory from
  `0x60000000` (`crates/io/or-io/src/constants.rs`). `IoError`
  (`crates/io/or-io/src/error.rs`) is this machinery's error.
- ⚠ `ErrorMode::Lenient`, the default, succeeds with less than the file holds:
  a dictionary skips an entry it cannot read. A read path with no compiled
  fallback reads `Strict`.

## The app

`src/main.rs` runs `app::run()`, or the headless self-check with
`--self-check`; `src/app/` drives the editor and the settings window over one
`SharedGpu`, `src/render/` bridges the scene to `or-renderer` (and `src/render/game_scene.rs` makes the open drawable the surfaces Shaded mode draws, and `src/render/dressed.rs` holds what the Wardrobe hands the viewport to draw for a ped that is no document),
`src/platform/` holds file dialogs, drag and drop, file associations, the
Recycle Bin, the clipboard's file list and the probe for the libraries the app
loads on Linux,
`src/video.rs` previews a `.bik` and plays its sound, Bink 1 and Bink 2 alike, through
`or-media`'s own decoders, `src/bink.rs` reads a movie's header and holds the install's
DLL that the Windows test comparing the Bink 2 decoder to it loads, and
`src/audio.rs` plays, through `rodio` on every platform, exports and replaces an
`.awc`'s streams, decoding them through `or-media` and re-encoding a
WAV into each stream's own codec, PCM16, ADPCM or MP3, through
`AwcFile::with_stream_replaced`, which also rebuilds a multi-channel stream's
shared blocks.

⚠ A call that reaches past the app, a file dialog, the clipboard, the file
manager, the Recycle Bin or the registry, goes through `src/platform/`, which
hands it to a stand-in that logs what it was asked in a test build and in a
self-check (`stand_in_for_the_desktop`); one made anywhere else reaches the
owner's desktop from a `just shots` click, and
`nothing_outside_the_platform_layer_reaches_past_the_app` refuses the ones it
knows.

### Where the app's behaviour lives

`AppState`'s methods are split across `src/state/` by concern, each module with
its own `impl AppState`. The struct is deliberately flat: its fields are mostly
handles to types with their own modules, and nesting them would make borrows
harder in an immediate-mode UI (`the_shell_is_flat_rather_than_deep`).

| Module | Owns |
|---|---|
| `mod.rs` | construction, the document list, the per-frame tick, dirty tracking |
| `file_io.rs` | opening and saving, on worker threads |
| `closing.rs` | closing documents, a new session, opening a project and exiting, each asking before it drops unsaved work |
| `reload_open_file.rs` | a recook, or an answer to the changed-on-disk question, reading again every tab open on a file: in place, dropping the steps that hold its old contents and the previews drawn from it |
| `pending_prompts.rs` | the edit gate's verdict, and the prompts a write into an install or the open of an Enhanced file waits on |
| `tabs.rs` | the tab strip's bookkeeping: which document has a tab and its name, the preview tab a single click in Files opens, the order tabs were used in for `Ctrl+Tab`, and the closed tabs `Ctrl+Shift+T` reopens |
| `document_io.rs` | bytes to payload and back as free functions, because the parse runs on a worker and must not touch app state |
| `payload_handlers.rs` | one `PayloadHandler` per resource format: parse, serialize, a blank document |
| `kinds.rs` | one `KindInfo` row per `DocumentKind`: noun, tooltip, glyph, colour, default workspace |
| `capabilities.rs` | what the editor can do with a kind (below) |
| `pack_edit.rs` | editing a described scalar in a document's bytes |
| `plugins.rs` | the embedded plugin host: the plugins directory, consent kept in the preferences, the edit mode as the write gate, and every refusal kept to show; its child `surface` holds the plugins that stay loaded: what each registered, the frames it describes, the settings kept in the preferences, the overlay it draws over the open document and the clicks it takes |
| `selection.rs` | the focused row every single-row reader sees, and the rows a multi-select adds |
| `edit_history.rs`, `undo.rs`, `meta_edit.rs`, `rpf_undo.rs` | each document's own history of edits and where its file stands in it, undo and redo through it, and a metadata edit and an archive session's edit as a step holds them. An edit records its step through `AppState::record`; `Document::mark_edited` stands for an edit no step holds, and empties the history |
| `bone_numbers.rs` | a bone's name, position, rotation and scale as one edit of its skeleton: the bone a selection names, the rotation's Euler order, and the step it records, which swaps the whole skeleton, and each skeleton of a fragment equal to it, in one step |
| `animation_session.rs`, `preview_scene.rs`, `viewport_camera.rs` | clip preview, what the viewport shows, one camera per open document |
| `rpf_session.rs`, `rpf_tree.rs`, `extract_job.rs`, `extract_planning.rs` | an archive's tree, lazy extraction, edits, extraction to disk, opening what an extraction names on workers |
| `clipboard/` | copy, cut and paste: rows held as the files a paste and Explorer read, a field of the inspector's grid whole, and records of a metadata array |
| `jobs.rs` | every background job in one list, read from its own controller each frame: what it is doing, its progress, its Cancel and what it left; and the files the jobs read, which a save, a delete, a move or a rename on disk and a defragment ask about first |
| `workbench/` | the open project as the Project workspace shows it: a worker's scan of its files and `.src` folders and the plan of each build (`scan.rs`), a saved source recooked into the shadow folder by a watcher and a worker (`cook.rs`), game files imported into the project on a worker with the report they leave and Undo import (`imports.rs`), builds as a job (`builds.rs`, which reads a clothing pack's build back against the pack, and `build_cooker.rs`, which says what each `.src` folder cooks to), the problems (`findings.rs`, the clothing engine's among them with the fix and the waiver each offers), the project's commands (`commands.rs`), the edits the Builds window makes to the builds and the locks of its matrix (`build_edits.rs`), how Locate asks for a folder or a file and a copy of a source is given an id (`members.rs`), the steps every change to the project is, kept beside it for Undo, Redo and a rollback (`project_history.rs`), the picture the quick look draws, read on a worker (`glance.rs`), the one rule and the plurals every count is said by, the one count of errors and warnings and the one name of a content and of a build (`tally.rs`), linked content (`links.rs`), the strip of workspace buttons (`strip.rs`), the Weapons screen's packs, selection and edits, each a step of its own Undo (`weapons.rs`), and the assembled weapon's scene, read on a worker (`weapons_scene.rs`), the Particle screen's packs and the effect it shows, with the commands that give a project particles and add files to them (`particles.rs`), the Wardrobe's packs, selection and the worker that runs its edits one after the other (`wardrobe.rs`), the edits that are each a step of Undo (`edits.rs`), the garments picked in the list and what the keys do with them (`wardrobe_select.rs`), what makes them recoverable (Redo, the one Undo of the Wardrobe workspace, the lock on a pack another window edits, the card of a pack that does not read, the versions of a pack: `wardrobe_recovery.rs`), Fix all safe, the repair of a model and the texture optimiser (`wardrobe_fix.rs`), the maps, the budget plan with its pictures, the completion of materials and the folders bound to slots with the watcher that puts a saved picture in (`wardrobe_textures.rs`) and the sentences of a drop and of what an edit moved (`wardrobe_words.rs`), the dressed ped it shows, whose files are read on a worker (`dressed.rs` and `dressed/read.rs`) and whose scene the viewport draws, with the way it is seen (`dressed/view.rs`), the key light it is seen in (`dressed/light.rs`), a prop carried to the bone of its anchor and skinned to it (`dressed/props.rs`), the clip it plays on the skeleton of the install (`dressed/motion.rs` and `dressed/clips.rs`), each garment alone for a picture (`dressed/alone.rs`) and the catalogue of pictures a worker draws of the pack (`dressed/catalogue.rs`), and the watch on the folders of those packs that asks for a rescan when a file changes, in place of the clock (`watch.rs`) |
| `front_door/` | what a launch opens and how a project is begun: the start page raised over the shell, the recent projects as cards read on a worker (`cards.rs`), the New project dialog's templates and the one plan its tree and its write follow (`new_project.rs`), the offer to make an opened folder a project and the preview of what the folder holds, read on a worker (`adopt.rs`), the template files the dialog lists, the project made on a worker while the dialog waits (`mod.rs`), the count and the plural a sentence of another crate is read with (`words.rs`), and the question a second project asks while one is open |
| `compare/` | a file compared with another, or one in the mods overlay with the game's own copy: read and lined up on a worker, and opened as a document of its own |
| `mod_manager.rs` | the active install's mods, the load order saved for it, the winner of two mods writing one path, and moving a mod between `mods` and `mods_` once the edit gate permits and the user answers |
| `install_lifecycle.rs`, `game_discovery.rs` | choosing a game and finding installs |
| `explorer.rs`, `navigation.rs` | archive-tree sorting, favourites, back and forward, and the walk that opens the archives on a path before revealing, opening or extracting what it names |
| `gta_vi.rs`, `rpf9_scan.rs`, `rpf9_list.rs`, `rpf9_kept.rs`, `gta6_install_scan.rs` | a GTA VI archive opened as what a scan finds, a slice at a time on a worker; which rows of the list show and which an extraction takes; a whole scan kept in the configuration folder; every archive of a folder scanned as one job |
| `ttpf_document.rs`, `ttpf_reshape.rs`, `gta6_bcpack_calibration.rs`, `gta6_texture_index.rs`, `gta6_dictionary_index.rs` | a texture pack as a document, a texture reshaped, the coder's tables fitted as a job, and the packs and dictionaries indexed by texture key so a drawable's key opens what holds it |
| `gta6_names.rs`, `gta6_install_harvest.rs`, `gta6_texture_naming.rs`, `gta6_texture_names_file.rs` | the harvested vocabulary served to every view, the harvest of an install as a job, the naming rules run on a worker, and a file of names the user picks |
| `gta6_resource.rs`, `gta6_resource_save.rs`, `gta6_edit.rs` and the other GTA VI edit modules beside them | a scanned body's facts, its edits as steps of undo, and its save as a loose resource through the `edited_sections` cell |
| `gta6_map.rs`, `gta6_map_edit.rs`, `gta6_map_links.rs`, `gta6_world.rs`, `gta6_world_fill.rs` | a scanned map on the Map screen, its entities edited, its links, and the lights, lane graph and other maps of an archive kept for the overlay |
| `gta6_shaders.rs`, `gta6_effects.rs` | the effect containers read and the programs a drawable's effects run translated on a worker, once a session, into a pack held in memory |
| `gta6_dialogue.rs`, `gta6_text.rs`, `gta6_audio_links.rs` | dialogue lines, radio and texts indexed from the open scans; a text table edited and saved; what audio game data links to |
| `file_index.rs`, `vfs_session.rs` | every file of the install and of each folder and archive open in Files, nested archives included, walked off the UI thread: what Go to File and the search of every archive read |

### Panels, docks and panel state

Panels are a registry keyed by a string id (`src/ui/panel_registry.rs`), and
`src/ui/panels/registry.rs` registers the built-in dock panels, and a plugin that
stays loaded registers its own as `plugin.<plugin>.<panel>`
(`src/ui/panels/plugin_panel.rs`); settings (its
own window) and the command palette are not dock panels. A large panel is a directory with a thin
`mod.rs`, as `text_editor/` and `meta_editor/` are, and `src/ui/timeline/` is
the format-agnostic timeline both previews use.

- `src/ui/dock_layout.rs` persists a dock arrangement and the workspace on
  screen, which the next start opens in, versioned and flushed on close; a
  saved id no panel provides keeps its tab as a `MissingPanel` placeholder. ⚠ A `PanelId` is `&'static str`, so an id read from a file is
  interned for the life of the process and the file decides how much leaks,
  which is why tabs per workspace are bounded. And a `DockState` not yet laid
  out holds infinite rects, which `serde_json` writes as `null` and then
  refuses, losing every other preference in the file; the geometry is flattened
  on capture and the layout read through a `serde_json::Value`.
- Panel scratch state lives in the `PanelStore` of `crate::ui::panel_state`,
  never in an `AppState` field, so a runtime-registered panel keeps state across
  frames (`a_panel_registered_at_runtime_keeps_state_across_frames`). `get`
  serves a value touched at one point, `take` and `put` state held across a
  long draw; but a taken slot reads as `Default` to anything reached before the
  put-back, which then discards what that code wrote, so a draw that dispatches
  into other modules, as the Files and Outline panels' do, uses `get`
  (`a_taken_slot_reads_as_default_and_the_put_back_wins`).
- **Keys by where they sit, by place and by preset**
  ([R70i](../direction/Roadmap.md#r70i-selecting-and-moving-in-the-world),
  [R70v](../direction/Roadmap.md#r70v-the-keys-settings-and-guide-of-a-mapper)). The binding by
  where a key sits is built, and no tool is on it. A row of the command table says whether its keys
  name a position (`by_position` in `src/ui/commands/table.rs`): such a binding answers the key at
  that place whatever it types, and Shift is part of it, so a digit and Shift with it are two
  bindings (`the_fourth_key_of_the_digit_row_answers_by_its_place_on_a_french_keyboard`,
  `a_digit_with_shift_and_the_digit_alone_are_two_commands_by_position`, on presses built by hand,
  never on a French keyboard in a window), and a panel that reads its own keys reads the place from
  the press egui hands it (`a_panel_reads_a_key_bound_by_position_from_the_place_its_press_states`).
  A hint and Settings name such a key by what it types once a press has shown it, and before that by
  its place on a US keyboard, said as such
  (`a_key_bound_by_position_is_named_by_what_it_types_once_a_press_has_shown_it`,
  `a_hint_reads_a_key_bound_by_position_as_the_keyboard_in_use_types_it`,
  `src/ui/shortcuts/position.rs`), and Settings takes the place of the key pressed for such a
  command (`a_key_captured_for_a_command_bound_by_position_is_the_place_pressed`). No row of the
  table says so yet and no default changed: the tools, their keys and the Map scene's own keys as
  rows of the table come with the screen, and until a row says so no check draws a hint or a row of
  Settings for such a command. ⚠ Two limits stand. egui names a digit of the numeric keypad as the
  digit row's, so the two are one place, and the keypad is only kept from renaming the row
  (`the_numeric_keypad_does_not_rename_a_key_of_the_digit_row`). A conflict is judged on a US
  keyboard, where Shift and the key at a place meet the character that key then types
  (`a_shifted_key_by_position_meets_the_character_a_us_keyboard_types_there`); a keyboard whose key
  at a bound place types another binding's character makes two bindings meet that the check does not
  see. The rest is not built. The Map screen's manipulator, snaps and typed fields work over boxes
  ([built](../direction/Workbench_UX_Design.md#what-the-map-screen-builds)), and no command targets
  a placed thing (`src/ui/commands/target.rs`). Presets of keys and the place of a binding are
  built, ahead of the row they were planned in, because the tools of
  [R70i](../direction/Roadmap.md#r70i-selecting-and-moving-in-the-world) need both. Settings ▸
  Keyboard shortcuts offers a preset (`src/ui/shortcuts/presets.rs`): OpenRage's own, which changes
  no command, and CodeWalker's, which holds only what a command of today's table can take, Ctrl+N
  for New project…; the keys of its tools and of its camera wait for the commands R70i and
  [R70c](../direction/Roadmap.md#r70c-the-gta-v-world-streamed-and-drawn) add. A preset changes the
  commands it names and no other, a key the person chose stays over it and after it, the choice is
  kept in the preferences, and the page says how many commands it changes and how many keep a key of
  the person's
  (`a_preset_changes_the_commands_it_names_and_no_other_and_openrages_own_puts_them_back`,
  `a_key_the_person_chose_stays_under_a_preset_and_after_it`,
  `a_preset_is_picked_from_a_named_control_and_says_how_many_commands_it_changes`). A conflict is
  one key for two commands read in one place: a row of the command table names its place (`Place` in
  `src/ui/commands/table.rs`), two panels share a key, and a command the key dispatcher reads shares
  with none (`two_commands_share_a_key_only_when_the_table_reads_them_in_two_places`,
  `no_two_commands_of_the_table_read_in_one_place_share_a_key`,
  `no_preset_binds_one_key_to_two_commands_of_one_place`, and on the page
  `the_page_gives_a_panels_key_to_another_panels_command_and_refuses_it_to_the_dispatchers`). A key
  that two panels share is refused to a command of the dispatcher with each of the two named, and
  Overwrite unbinds both and says so
  (`the_page_names_every_command_a_key_is_taken_from_and_overwrite_unbinds_each`). Restore all
  defaults is offered under a preset and returns to OpenRage's own
  (`restore_all_defaults_is_offered_under_a_preset_and_returns_to_openrages_own`), and a command set
  by hand to the key its preset gives is the preset's again
  (`a_key_set_to_what_the_preset_gives_is_no_key_of_the_persons_own`). Seen on `just shots` screens,
  not in a window. That presets exist is the default of the plan's ruling on the keys, which the
  entry of 2026-10-03 left to the owner
  ([Project_Context.md](../direction/Project_Context.md#the-ux-reviews-plan-is-the-apps-next-work-2026-10-03)).
  Left: the page of a mapper's settings, the hover of a panel's title, the map rules' sentences in
  the catalogs, the tour, the chapter of the guide and the template. A library crate's sentences are
  English, and `Template::Maps` opens on the Map screen.
- Every command is a row of `COMMANDS` in `src/ui/commands/table.rs` (the checks that enable one are `src/ui/commands/predicates.rs`): its id,
  label, category, default bindings, an enabled-when that says why not, and
  what it runs. A row also says where its keys are read (`Place`: the key
  dispatcher, or the panel that reads them itself under the pointer or with
  focus) and whether they name where a key sits and not the character typed
  (`by_position`). `src/ui/shortcuts.rs` holds a press against the bindings: a
  key the person chose first, then the chosen preset's
  (`src/ui/shortcuts/presets.rs`), then the table's; two commands share a key
  only when both are read by panels, in two places (`taken_by`), and
  `src/ui/shortcuts/position.rs` names a key bound by where it sits by what it
  types once a press has shown it. A command that acts on something (`On`) takes a `Target`: what
  a context menu was opened on, else the focused panel's selection
  (`src/ui/commands/target.rs`). The menu bar, the context menus, the command
  palette, Settings ▸ Keyboard shortcuts and the key dispatcher read it, and
  `src/ui/commands/mod.rs` adds what recent files, workspaces, edit modes,
  panels and plugins offer, and the commands of Tools, Help and
  Notifications are `src/ui/commands/tools_and_help.rs`. ⚠ A context menu's command runs once the frame's
  rows are drawn, so it never changes the tree the menu is drawn from.
  ⚠ `prefs.json` keeps a user's binding under the command's id, so an id is
  never renamed.
- Every workspace has one job per region: on the left Files
  (`src/ui/panels/hierarchy/files.rs`, each install, folder and archive open as
  a tree) above the Outline (`src/ui/panels/hierarchy/outline_panel.rs`, the
  active document's own tree), its own centre, and Details on the right: the one panel of what is selected on the screen
  in front, a task screen's row naming what it draws there (`src/ui/task_screens.rs`). A rail at the
  left edge (`src/ui/panels/rail.rs`) runs the commands that lead to Home, Game, Project and
  Automate, and the screens of a project are tabs before its files' (`src/ui/panels/document_tabs.rs`).
  Every tab bar shows its titles, and the focused panel's is in the accent:
  the panel Maximize shows. A click, Back, Forward or a breadcrumb in Files
  makes its document active only where the workspace on screen shows that kind
  (`activate_if_shown`), so moving through Files never switches the workspace;
  a click on a file of a kind the workspace shows opens it in the preview tab,
  except in Browse, whose centre is the Files view
  (`src/ui/panels/hierarchy/files_view.rs`): the folder where the user last went
  in Files (`AppState::files_view_folder`, a file's being its folder), in
  Details or Icons, beside the preview pane. Its pictures, a texture's image and
  a model's render, are read and decoded off the UI thread
  (`src/state/files_view/pictures.rs`), and a model's geometry reaches the
  thumbnail renderer with its request, since no open document holds it.
  ⚠ A panel drawn inside the dock changes a copy of the arrangement that the
  drawn one then replaces, so it shows another panel through
  `WorkspaceDocks::reveal`, which applies once the dock is drawn.
- `Workspace::Project` is the neutral view of an open project and is offered only
  while one is: `commands::workspaces` lists the six workspaces of a window with
  no project, seven with one and eight with one that holds clothing, and the strip draws what fits, then More. Its panels are
  `src/ui/panels/project/` (Project, Assets, Properties, Problems and Build, the buttons of a finding that Problems and Properties share in `fixes.rs`, what the Build tab says of the acceptance sheet in `in_game.rs`, and the Builds window in `builds_editor.rs`) with History
  (the project's events there), Jobs and Log, and each reads what
  `AppState::workbench` holds and never touches the disk. It handles no kind of
  document, so opening an asset moves to the workspace that suits it. The commands are
  `src/ui/commands/project.rs`: the Project menu, Ctrl+B for a build and Ctrl+1 to Ctrl+9 for a
  workspace, in the order of the buttons shown and then the rest. Import game files…, Import folder…, Import into
  Project (a row's menu in Files), a drop on the Project workspace (`DropTarget::Project`, planned in
  `src/state/os_drop.rs`; the dock notes the workspace's area and `src/ui/os_drop.rs` reads it) and Add
  images as a textures source… start `src/state/workbench/imports.rs`,
  a job in the status bar with a count of files, a bar and a Cancel (`or_source::game_import::Watch`) whose report is the window
  `src/ui/panels/import_report.rs`, drawn with the
  other questions; Undo import removes the folders that import made while none was changed. Import built pack… and a drop of a built pack ask `Question::Reopen` (`src/ui/panels/project/reopen_question.rs`) and run `src/state/workbench/reopen.rs`, a job of the same list with a Stop (`JobKind::ProjectReopen`) that ends in the window `src/ui/panels/reopen_report.rs`; it writes a new folder and never the pack, so it has no Undo. ⚠ A project's
  `shown_workspaces` key holds the buttons it pinned, and a build of the editor that does not read it
  carries it.
- The table of task screens is `src/ui/task_screens.rs`: one row (`TaskScreen`) for each screen that belongs to a kind of content, read through `all`, `of_workspace`, `of_kind` and `of_template`. A row holds the workspace and the kind of content, the words of the content and of its count, what Open in answers for other content and for one of its rows, the heading of the empty state and the command that adds the content, the hint of a drop and its command, the commands the Project menu and the Assets landing list, the layout and the panels a saved layout must keep, the template that starts it with the buttons it pins and what its chip on the start page says, the folder at which a data source of the content ships, how a content is counted and how a start page card counts it, and how the screen is made to show a content. The strip, the palette, Content, Open in, the empty state, the start page, the Project menu, the status bar, the saved layouts and the counts read it, and none of them names a kind of content or a task screen. ⚠ A second task screen is a variant of `Workspace` named in `Workspace::ALL`, a row and its panels. `no_site_matches_on_the_kind_of_a_task_screen` fails when a place outside the table matches on the clothing content (the files that read a pack with the Wardrobe's engine are listed with the reason) or on the Wardrobe, `a_second_task_screen_is_one_row` adds a stand-in row for the length of one test, and `the_task_screens_draw_as_they_did_before_the_table` compares the Project and Wardrobe screens with a record of what they drew (`OPENRAGE_RECORD_SCREENS=1` records it again when a change to them is meant).
- `Workspace::Wardrobe` is the first row of that table: the screen of a kind of content, offered only while the project holds clothing; it follows a separator in the strip and takes the last key. Its panels are `src/ui/panels/wardrobe/` (the garments, the header with the filter, whose commands `header::bar` draws in the row of tabs, the chips and the line of figures under the list, and the garment selected; the rows and the slot headings of the list are `rows.rs`, its keys, menus and quick look are `keys.rs`, `menus.rs` and `quick.rs`, and the Garment panel keeps its place form, its tags and group, its texture rows and its problems in `garment/`; the dressed ped is `preview.rs` for the rows, the view and its notes, `wear.rs` for who wears it and which version is shown, `tools.rs`, `parts.rs`, `light.rs` and `views.rs` for how it is drawn and what plays on it, `overlay.rs` for the helpers under the view and what is drawn on it, `tiles.rs` for the strip under it and `kit.rs` for the controls they share); the bottom of the workspace is the Project workspace's Problems, Build, History, Jobs and Log. A drop on the Wardrobe's panel is `DropTarget::Wardrobe` and is Add clothes (`src/state/os_drop.rs`). The commands are `src/ui/commands/wardrobe.rs`, and a toast can carry a button (`ToastAction`), which is how a drop offers Undo.
- `Workspace::Weapons` is the second row of that table: the screen of a kind of content, offered only while the project holds weapons, after the Wardrobe in the strip. Its panels are `src/ui/panels/weapons/` (`list.rs` the weapons grouped and filtered, `weapon.rs` the weapon selected with the rows of its typed info in `fields.rs`, and `preview.rs` the assembled weapon, which the viewport draws through the scene of `src/state/workbench/weapons_scene.rs`); the bottom of the workspace is the Project workspace's. A drop on the list or on the assembled weapon is `DropTarget::Weapons` and is Add weapons (`src/state/os_drop.rs`), the commands are `src/ui/commands/weapons.rs`, and the state, whose edits are each a step of Undo, is `src/state/workbench/weapons.rs`.
- `Workspace::Particle` is the third row of that table, and the first of a workspace that is also the editor of a kind of file (`edits_files`): it is offered with no project, keeps its name and its own saved arrangement, and while a project holds particles it gets a second one, `SavedLayouts::for_a_project`, which is the row's layout and holds the Effects panel (`src/ui/panels/particles/`, the effects of every particle file of the pack by file, with the filter and the two buttons). A click on an effect cooks the source if it is not cooked and opens the cooked file (`show_effect`), and a save of that document goes into the source (`or_source::particles::write_back`). A drop on the Effects panel is `DropTarget::Particles` and is Add particle effects, the commands are `src/ui/commands/particles.rs`, and the state is `src/state/workbench/particles.rs`.
- `Workspace::Map` is the fourth row of that table: the screen of a kind of content, offered only while the project holds maps. Its panels are `src/ui/panels/maps/` (`list.rs` the sources of the set shown with the entities and archetypes under them, `scene.rs` the set as boxes in the viewport with the tools and the manipulator, `plan.rs` the set seen from above, and `entity.rs` the selection, every number a field that takes what is typed). The viewport is not changed to draw it: the scene panel builds the lines of the frame (`src/render/map_overlay.rs`), keeps them in `state.workbench.maps.overlay`, and the viewport's list of lines in front, the one plugins draw through, takes them with `map_overlay::append`, while `step_viewport_camera` hands the Map workspace its own camera, as it does the Wardrobe and the Weapons. A drop on the scene, the plan or the list is `DropTarget::Maps` and is Add map files (`src/state/os_drop.rs`), the commands are `src/ui/commands/maps.rs`, and the state, whose edits are each an operation of `or_maps::operation` and a step of Undo that keeps the XML it wrote, is `src/state/workbench/maps.rs` (the scan, the scene and the selection) and `maps_edit.rs` (the edits and their steps).
- The start page is `src/ui/panels/start_page.rs` (the five ways in, the goals of a first launch and the
  recent projects as cards) over `src/ui/panels/game_picker.rs` (the installs, the recent files and the
  keys), drawn by the Document panel while nothing is open and by `src/ui/dock.rs` in place of the
  workspace while `FrontDoor::raised` says it is raised over it. The New project dialog and the
  question a second project asks are `src/ui/panels/new_project.rs`, drawn with the other questions,
  and the banner over an opened folder is `src/ui/panels/adopt_banner.rs`, drawn by the Files view.
  The commands are `src/ui/commands/front_door.rs`. ⚠ `platform::new_window` starts the second
  window as a process, so a self-check and a test tell its stand-in instead; two processes share
  `prefs.json`, and `save_prefs_to` writes over the file only the keys this one changed.
- A workspace with nothing of its own, nothing active or an active document of
  a kind it does not handle (`has_nothing_to_show`), draws its empty state from
  `src/ui/panels/empty_state.rs`: in the Document panel where the workspace has
  one, with the inspector naming only what is missing, and otherwise in the
  inspector. Outside Browse an install is the tree beside the work, and in Data
  a model is another workspace's, so a panel that drew it there would be
  drawing another kind's view.
- Drawing a panel never changes a document
  (`opening_a_file_of_every_kind_and_drawing_every_panel_modifies_no_document`). An
  egui `DragValue` rewrites the value it is handed with no input, a NaN to an
  infinity and a value past its range to the range's end, and reports that as a
  change, so a panel draws a document's number through `ui::panels::drag_value`,
  which writes only what the user changes. A value the file leaves out is shown
  as unset, and only an action the user takes writes a default.
- A question the user answers before going on (unsaved changes, a delete, a
  name already taken) is drawn through `ui::panels::question`, an egui modal
  that keeps clicks from the shell behind it, and a key command or a dropped
  file waits for its answer
  (`every_question_leaves_the_shell_behind_it_inert_until_it_is_answered`).
  ⚠ An `egui::Window` leaves the shell working behind it.

### What a format costs the app

A paged format is three registrations and no lines in `src/`: a `FormatHandler`
in `src/state/format_handlers.rs` (which extensions name it), a
`PayloadHandler` (how to read, write and create it) and a `FormatCapabilities`
row (what the editor can do with it). A `kinds` row is optional; without one
the row is visibly provisional
(`a_format_nobody_described_gets_an_obviously_provisional_row`).
`src/state/adding_a_format.rs` declares a format inside itself and walks
detected → named → opens → described (`a_synthetic_format_is_fully_usable`); it
lives in `src/` because the `openrage` package has no library target. The opens
rung needs a payload of the requested kind, since counting a parse error as
reaching it would pass a handler that always fails.
`every_registered_resource_extension_opens` reads the live format registry, not
`ResourceType::extensions()`, and every kind's default workspace shows it
(`a_kinds_default_workspace_can_show_it`), or activating the document bounces
it to another workspace. A new format gets a document view, not a workspace:
the Document panel (`src/ui/panels/document_view.rs`), the centre of Browse and
Data, draws a kind with no editor of its own from its inspector body, and the
inspector then shows only that document's facts
(`a_kind_with_no_editor_of_its_own_is_drawn_once_in_the_centre`).

`DocumentKind` names `Ypt`, `Ytd`, `Ydr`, `Ydd`, `Yft`, `Ybn`, `Yld`, `Ycd`,
`Yed`, `Yfd`, `Ynd`, `Ynv`, `Ypdb`, `Fxc`, `Gxt2`, `Bik`, `Awc`, `HeightMap`,
`DistantLights`, `WaterMap`, `WorldBlanket`, `NavPrecalc`, `DatTable`, `Rel`, `NameTable`, `Gfx`, `MoveNetwork`, `Dds`, `Ttpf`, `Rpf`, `Rpf9`, `Gta6Anim`, `Gta6Drawable`, `Gta6Text`, `Gta6AudioData`, `Gta6Expressions`, `Gta6Save`, `GameRoot`, `Folder`, `Text`, `Meta`, `Hex` and `Compare`, and `Registered`
carries the `ResourceType` of a detected format the enum does not name
(`src/doc_counts.rs` holds the list). Only the format's own
rows look inside `DocPayload::Registered`, by downcasting to the reader's type
(a `.ysc`'s `ScriptDocument` holds the listing the inspector shows and
its decompiled source in the Data workspace), and the generic panels read the
document's resource image. A kind is generation-agnostic, so a Gen9 payload finds its
format's row (`an_enhanced_payload_shares_its_formats_row`). A file no kind
names, or a binary `.dat` no reader opens, opens as `DocumentKind::Hex`: its
bytes, read-only, in the inspector, and never saved. A file no kind names whose
bytes are text (UTF-8 with no NUL in its first 8 KiB, or a UTF-16 byte order
mark) opens as `DocumentKind::Text` instead. `DocumentKind::Compare` is no
file's: it is two files compared (`src/state/compare/`), opened and never saved.

A GTA VI kind costs more than a paged format, because it has no `ResourceType`, no extension and no
declared root: `Rpf9`, `Ttpf`, `Gta6Anim`, `Gta6Drawable`, `Gta6Text`, `Gta6AudioData`,
`Gta6Expressions` and `Gta6Save` each take arms in `src/state/document_io.rs`, a `KindInfo` row and a
capability row, and a scanned body is opened from a row of its archive's scan, by its offset, where
another kind is opened from a path. One kind serves the headerless resources: a drawable, a fragment, a
bound and a dictionary are all `Gta6Drawable`, whose payload is whichever resource the reader read. A GTA VI
metadata body is a `Meta` document, and its mission state tables, its anim scene's timeline and its
map on the Map screen are views over that document and no kind of their own. A kind that saves its
edits returns them from `edited_sections`, and `src/state/gta6_resource_save.rs` writes them as a
loose resource.

`src/state/capabilities.rs` is the table, builtins plus a process-global behind
a lock, keyed by `kind` because a `FormatId` cannot name an archive, a text
document or a metadata document. Every other field is an `Option` cell, and
`None` is the usual answer:

| Cell | Answers |
|---|---|
| `selection_summary` | the status bar's line for a selection |
| `outline` | the Outline panel's tree |
| `has_outline_children` | whether this document has anything to expand |
| `inspector` | a hand-written inspector body |
| `meshes` | the viewport's geometry for the document alone |
| `scene_meshes` | the geometry it adds to a composed preview scene |
| `lines` | wireframe overlays |
| `bounds` | the sphere the camera frames |
| `textures` | the textures the gallery lists |
| `models` | the models the gallery lists |
| `texture_for_selection` | which texture a selection names |
| `texture_at` | fetching a listed texture for the gallery and the preview |
| `texture_named` | the viewport's material lookup by name |
| `mesh_sources` | what the preview-scene picker offers |
| `skeleton_sources` | which skeletons a document offers |
| `skeleton_data` | the skeleton behind one of those sources |
| `edited_sections` | the sections a GTA VI resource document holds edited, which a save writes |

- A later registration replaces an earlier one within its
  `format_handlers::Tier`, and lookup walks preferred, standard,
  fallback. ⚠ A registration replaces the whole row and never merges, so an
  override built from `FormatCapabilities::none` clears every cell it does not
  set; start from the current row
  (`a_later_registration_replaces_an_earlier_one_in_the_same_tier`). The title
  and panel registries refuse a duplicate id instead (`plugins/README.md`).
- Cells that look like one decision are two, and a test pins each difference:
  an empty dictionary has an `outline` and no `has_outline_children`; an
  unnamed dictionary entry is drawn alone by `meshes` and with its siblings by
  `scene_meshes`; `texture_named` leaves out `.ytd`, a global fallback
  (`a_material_lookup_is_not_the_same_set_as_a_browsable_dictionary`);
  `mesh_sources` leaves out `.ybn` and `.yld`
  (`two_formats_can_be_drawn_into_a_scene_and_never_offered_to_one`).
- A fragment's texture tiles are addressed by index under
  `FRAGMENT_ENTRY_HASH`, as a `.ydd` garment's are; the `.ydr` encoding makes
  tiles the preview cannot read
  (`a_fragments_tile_is_addressed_the_way_its_outline_row_is`).
  `TextureResolution` has three cases, because a selection that is not a
  texture and a texture that is gone are different facts.
- A kind with no hand-written inspector or outline gets the described grid
  and `Outline` (`every_reachable_format_has_an_inspector_or_a_described_root`),
  which show a root's scalars and not a `.ypt` behaviour tree, so most named
  kinds keep a body of their own. The grid reads the built-in description of
  the format `or_io::gta5::formats` declares for the kind's extension, at the
  target the file's RSC7 version names, and needs no pack
  (`every_declared_format_and_target_gets_a_described_outline_and_summary`).
  Its two empty answers (no declared format, as for `.ypdb`; a version no
  declared root reads) stay distinct.
- An array of `DocumentKind`s drifts silently where a `match` would not, so the
  New menu is one derived list, `kinds::creatable`
  (`nothing_offered_would_refuse_if_clicked`); `Workspace::handles_kind` stays a
  `match`, because grouping is a product decision.
- A kind with no `ResourceType` still costs arms in `src/state/document_io.rs`:
  `GameRoot`, `Folder` and `Compare` have no bytes, `Rpf` needs a crypto provider, and
  `Text`, `Meta`, `Gxt2`, `Ypdb`, `Fxc`, `Bik`, `Awc`, `HeightMap`, `DistantLights`, `WaterMap`, `WorldBlanket`, `NavPrecalc`, `DatTable`, `Rel`, `NameTable`, `Gfx`, `MoveNetwork`, `Dds`, `Ttpf`, `Rpf9`, `Gta6Anim`, `Gta6Drawable`, `Gta6Text`, `Gta6AudioData`, `Gta6Expressions`, `Gta6Save` and `Hex`
  are not paged.

### What a kind of content costs

A kind of content is a typed family or a source form under it, a crate, one line in two lists, one row of a table and its panels. Weapons (wave P42) was the second, Particles (wave P43) the third and Maps (wave P44) the fourth, and the steps below are the order they were built in. Each step ends on a check that has been seen failing, and the next does not begin before it passes.

1. **The data.** Measure the owner's install read only (`OPENRAGE_GAME_ROOT`), and declare in `or-io` only what was measured, with the engine's names (D15) and as an owned value (D16): `meta::typed::weapon` is the example, each root declared in the PSC's dialect (`typed::weapon_types`) so it opens with no schema snapshot, and a record keeps what no member names so that a save loses nothing. The check is a suite that opens every file of the install, saves, compares the two texts element by element, saves the saved text again and holds the declared members to the install (`every_weapon_file_of_the_install_opens_saves_and_arrives_as_it_was`). It is install-gated, so it reports a skip in the skip ledger when no install is named and is never counted as a pass; a second suite with documents of its own runs everywhere. No game content goes into the repository, a fixture or a document, and what was not measured stays raw and is reported as unmeasured.
2. **The kind.** A `ModuleKind` variant with its folder and manifest in `or-project` (`container.rs`, `links.rs`, `template.rs`), the reload of the files a build wrote in `readers.rs`, and a crate in the shape of `or-addonped`: `model`, `layout`, `facts`, `metadata`, `rules` as data, `plan` as a `Contribution`, `verify`, `adopt` as a `Content` and a recogniser, then `import`, `edit` and a pure query where the kind has them. One line in `kinds` and one in `recognisers` of `or-content` register it, and `openrage-cli` verbs are one file. Build cells are not written: `cannot_ship` derives them from what the contribution hands over (`a_singleplayer_dlc_cell_is_derived_from_what_the_content_hands_over`), and a data source ships at the folder the row of the kind's task screen states.
3. **The row.** One `TaskScreen` in `src/ui/task_screens.rs` (the workspace, the kind, the words and hovers, the empty state, the drop, the menu, the layout, the template it starts, where a data source ships and what is held), a `Workspace` variant listed in `ALL`, a `Template` variant (`src/state/front_door/template.rs`), the commands (`src/ui/commands/weapons.rs`, with its `Action`s in `src/ui/commands/table.rs`), the state (`src/state/workbench/weapons.rs`, which `Scan` reads with the rest), the panels registered in `src/ui/panels/registry.rs` and the drop target of `src/state/os_drop.rs`. No other place may name the kind: `no_site_matches_on_the_kind_of_a_task_screen` fails one that does, and the row appears in the strip, Content, Open in, the start page, the empty state and the Project menu with no line of its own. A screen over a workspace that already edits a kind of file (the Particle workspace) sets `edits_files` and takes no new `Workspace` variant: the workspace keeps its name, its arrangement and its place in a window with no project, and the row's layout is a second arrangement used while a project holds the content.
4. **Every string, and its translations in the same wave.** Each sentence goes through `tr!`, `tr_n!` or `tr_key!` and has its entry in every catalog of `src/i18n/languages`, the French one with the no-break space before `:`, `;`, `?` and `!` and inside « and »; the sentences of the engine crate stay English ([Interface languages](#interface-languages)). `the_french_screens_clip_break_and_overflow_nothing_the_english_ones_do_not_at_the_smallest_window` draws the new screen in both languages, and it found a control that ran past the window in English too.
5. **Every control, with a name a screen reader speaks.** A row in the control inventory of [Workbench_UX_Design.md](../direction/Workbench_UX_Design.md#control-inventory) for each control, found by a walk of the screen (`weapon_controls` in `every_control_of_the_project_screens_has_a_row_with_its_place_and_its_reason`).
6. **The screens.** The record of the task screens is taken again and what changed is said (`the_task_screens_draw_as_they_did_before_the_table`), and `just shots` takes the screens at 1280 by 720 in both languages, since nobody opens a window.
7. **The documents.** The crate's section and the layer row here, the screen and its inventory rows in the UX design, what the design draft asked in [Workbench_Views_Design.md](../direction/Workbench_Views_Design.md), and the crate's name in the workspace table of `AGENTS.md`.

### Two address spaces for a selection

`src/ui/panels/selection_ids.rs` names a place in the parsed model and survives
an edit; `or_editor::PackPath` names a place in the file's bytes and survives
the description changing. Neither replaces the other. `Document::mark_edited` drops a
document's bytes at the first structural edit, deliberately, since drawing from
stale bytes is worse than drawing nothing
(`mark_edited_is_the_only_way_a_document_becomes_dirty`); most YPT selections
address a tree the app mutates; and a dictionary selection (`dict_entry_id`) is
keyed by name hash, which survives insertion. So only a description-derived outline
row selects by `PackPath`. What a YPT selection should be is open.

### Interface languages

Every sentence a person reads in the app goes through the mechanism in
`src/i18n/mod.rs`. The English text is the key, so English needs no catalog and
a sentence nobody has translated reads in English. A language is a `.po` file
of the part of gettext a translation needs: `msgid`, `msgid_plural`,
`msgstr`, `msgstr[N]`, `#` comments, and a header entry with `Language-Name`
and `Plural-Forms` (only `n != 1`, `n > 1` and the three forms of Russian are
understood). Nine catalogs are built in (`BUILT_IN` in `src/i18n/mod.rs`, the
files of `src/i18n/languages`), and the gates hold each complete; a file `CODE.po` in the `languages` folder of
the configuration directory is laid over the built-in one of that code, or is
a new language when no built-in has it, and is read once at launch
(`a_file_of_the_folder_lies_over_the_built_in_catalog_and_a_new_code_is_a_new_language`).
The Settings page, Appearance, Language, picks one or the system's, which is
the default; `AppState::set_language` saves it, applies it from the next frame
and refreshes what was said when it was read (the start page's cards). A test
build's system language is English, and a test that draws text pins its own
language with `crate::i18n::pin("en")` (a guard for its thread) so no other
test sees it. Chinese, Japanese and Korean have no glyph in the fonts the
program carries: `font_definitions` in `src/app/gpu.rs` adds the system's fonts
for them last, the one of the language in use first, and a window restyles
when the language changes.

**How a new string is added.**

1. Write the sentence in English with one of three macros, and nothing else
   around it:
   - `tr!("Add clothes…")` is a `&'static str`. With names it is a `String`:
     `tr!("Built {ago}: {size}", ago = ago(at), size = size(n))`. A name is a
     plain `{name}` of letters, digits and underscores, takes the value it is
     given once, and is written in lower case (an upper case name does not
     compile in the pattern the macro builds). `{{` and `}}` are the braces
     themselves.
   - `tr_n!("{count} file", "{count} files", n)` is a sentence that depends on
     a number, with both forms in English and `{count}` standing for the
     number. More names follow the number: `tr_n!("…", "…", n, folder = f)`.
   - `tr_key!("Open")` is the literal itself, for a `const`, a `static` or a
     table row (a command's label, a column's title) that cannot call a
     function. It is translated where it is drawn: `crate::i18n::tr(label)`,
     and a key drawn without `tr` stays English in every language
     (`the_empty_wardrobe_and_the_level_of_detail_are_said_in_french`).
2. Say the whole sentence. A sentence built from pieces (`format!` over a
   translated noun, a plural made by appending `s`) cannot be put in another
   language's order: give each sentence its own `tr!` or `tr_n!`, and let the
   words a number counts be a `tr_n!` of their own
   (`src/state/front_door/words.rs` has `counted`).
3. Add the entry to every catalog of `src/i18n/languages`, sorted by its English
   text, with the same names in braces under the same spelling, and one form
   per form of the language's rule for a plural. The French catalog's words are in
   [Interface_Languages.md](../guides/Interface_Languages.md#french).
4. Run `cargo nextest run --bin openrage i18n::` (the gates below), then
   `just ready`. Changing the English of a sentence changes its key, so the
   entry follows it.
5. Wrap every English word a sentence puts together, not only the sentence: the
   fallback of an `unwrap_or("…")`, a noun a `format!` joins and a word a
   `tr!` takes as an argument are each a `tr!`, or the French sentence holds
   English in the middle of it. Nothing flags a literal that has no `tr!`, so
   draw the screen in French once (`just shots` with a `language fr` step) before
   a wave is done.
6. Two places read the English and must not translate it. A text the code looks
   for in what a library crate wrote (`plain` in
   `src/ui/panels/import_report.rs`, which cuts the library's own words) is
   matched as the library wrote it, never with `tr!`: a catalog entry for it
   would change the text looked for and the cut would never match. A value the
   preferences or a project file keeps names its choice by a key of its own
   (`ThemeChoice::from_key` reads `Dark` and `Light`), never by a label, which
   is in the language in use.

**When the list of languages is read.** `AppState::new` reads the `languages`
folder before the preferences (`read_the_languages` in `src/state/mod.rs`),
because the first text any code asks for fixes the list of languages for the
process. Code that asks for a text while the preferences or a project are read
must come after that call
(`a_language_file_in_the_folder_is_listed_by_a_launch_that_has_saved_preferences`
runs a launch in a process of its own).

**What stays English** because a person does not read it as a sentence of the
app: ids and file names, format names and hashes, anything `tracing` logs, the
developer tools (the renderer, the particle and animation editors), the
sentences that a library crate returns (`or_wardrobe`'s and `or_weapons`'s rules and
`or_project`'s build reports, which reach the screen as they were written),
and a value a person typed. A `tr!` of such a text only adds an entry nobody
translates.

**What the gates check.** All of them are in the app's own suites, so
`just ready` runs them.

| Check | Fails when |
|---|---|
| `every_text_the_code_asks_for_is_in_every_built_in_catalog_and_every_entry_is_asked_for` | a text the code asks for has no entry in a built-in catalog, an entry has nothing in the code asking for it, an entry is empty or in the catalog twice, or a text is plural in the code and plain in the catalog (or the other way round) |
| `a_translation_keeps_every_argument_of_its_text_under_its_own_name` | a translation of any built-in catalog drops, adds or renames a `{name}` of its English text; a form of a plural entry may take the names of either English form |
| `every_built_in_catalog_names_itself_and_counts_in_a_rule_this_reader_knows` | a built-in catalog has no `Language-Name`, a `Plural-Forms` the reader does not know, an entry twice, or a plural entry without one form per form of its rule |
| `every_character_a_built_in_catalog_writes_is_in_the_fonts` | a built-in catalog writes a character that draws as a box (Chinese, Japanese and Korean are checked on Windows only, where the system has their fonts) |
| `a_text_with_a_name_in_braces_is_asked_for_with_the_value_of_the_name` | a `tr!` with braces and no arguments |
| `a_text_no_catalog_has_is_shown_in_english_and_the_log_says_so_once` | a text with no entry is not shown in English, or is logged each time it is drawn |
| `every_noun_a_count_is_said_of_has_its_own_words` | a noun that a number counts has no words of its own (`counted`) |
| `the_french_screens_clip_break_and_overflow_nothing_the_english_ones_do_not_at_the_smallest_window` | the Project workspace, the Wardrobe, the start page or the New project dialog (its gallery and the details of each template, opened on their own, since `Action::NewProject` is the Close all command) at 1280×720 has, in French, an ellipsis, a word broken in two, a text the edge of its panel cuts off or a control past the side of the window that English has not, or a dialog screen is not drawn at all |
| `a_language_file_in_the_folder_is_listed_by_a_launch_that_has_saved_preferences` | a launch that has saved preferences does not list a language file of the `languages` folder |
| `a_saved_theme_reads_back_whatever_language_is_in_use` | a preference is read back by a label in the language in use |
| `a_reason_is_put_plain_in_french_as_it_is_in_english` | the import report's reason keeps its jargon in French |
| `the_empty_wardrobe_and_the_level_of_detail_are_said_in_french` | the empty Wardrobe's drop sentence or the Level of detail combo stays English in French |
| `switching_to_french_redraws_the_project_the_wardrobe_the_menus_and_the_palette_at_once` | a switch of the setting leaves English on screen |

The scan that finds the calls (`src/i18n/used.rs`) reads every `.rs` file under
`src/` as the compiler reads a literal, tests included, so a sentence a test
asks for needs its entry too. The macros check at compile time that a text and
its names agree. The control inventory and the figure gates
(`src/ui/the_task_screens_draw_as_they_did_before_the_table.rs` among them)
pin English, so a language never changes what they read.

When a French sentence is too long for a control, shorten the French, not the
English: the layout test above names the text, and `just shots SCRIPT DIR` with
a `language fr` step shows it ([screenshots](../guides/Development_Workflow.md#screenshots)).
A menu is as wide as its widest row in every language but English
(`room_for_the_words_of_the_language`, `src/ui/mod.rs`).

## The src/ code-line gate

What the app crate's own suites hold about its size and shape:
`src/shell_budget.rs`, `src/suppressions.rs` and `src/state/tests.rs`, each a
`#[cfg(test)]` module run by `cargo test --bin openrage`.

### No file in src/ is over 900 code lines

A code line is one that is not a comment, a doc comment, blank, or inside a
`#[cfg(test)]` item (`no_file_in_src_exceeds_the_code_line_gate`). A file that
should not be split goes in the gate's `DECIDED` list with its reason; the one
entry is `ui/panels/text_editor/mod.rs`, whose virtualized paint-and-input loop
costs more split than whole. A file over the line and due to be split goes in
`NOT_YET_SPLIT`, pinned to its own count; that list is empty, and an entry that
stops breaching has to leave (`the_not_yet_split_list_shrinks_and_never_lies`).

The gate counts code because a raw-line gate points backwards. It would split
`src/state/capabilities.rs`, mostly documentation and tests, and pass
`src/ui/panels/inspector/ypt.rs` and `src/ui/panels/hierarchy/ypt/mod.rs`,
almost all code and untested: it taxes documentation and tests and rewards a
file for having neither. `src/state/kinds.rs` was the second example until its
one table row per document kind outgrew its notes.
`the_gate_reproduces_the_arguments_shape` asserts that shape rather than a
figure: `src/state/capabilities.rs` stays mostly not code, the YPT panels mostly code,
`src/state/capabilities.rs` is over the line raw and under it in code, and
something in `src/` still exceeds it in code.

Pins do not ratchet down automatically, because a gate that fails for a reason
the committer did not cause teaches people to raise the number; a pin is
lowered in the commit that earns it.
`cargo test --bin openrage shell_budget::report -- --ignored --nocapture`
prints the largest files beside their budgets; splitting redistributes code, so
the crate's total measures nothing.

### Every .rs file in src/ is declared by a module

A file no `mod` declares is compiled and tested by nothing, and still looks like
part of the tree. `every_rs_file_in_src_is_declared_by_a_module` follows the
`mod` declarations from `src/main.rs` and `src/cli_main.rs` as rustc does,
`#[path]` included, and fails on a file under `src/` that none reaches. It reads
a declaration only at column 0, where rustfmt leaves one, so a `mod name;`
indented inside an inline module is not followed.

### AppState holds application state; no panel reserves a field on it

`no_panel_reserves_a_field_on_app_state` fails on an `AppState` field whose
type is declared under `src/ui/`; application state that is spelled in the UI
layer is listed in `UI_TYPED_BY_DESIGN` with its reason, and panel state goes in
the `PanelStore`. The gate keys on the type, so it cannot see a primitive
field: it finds a panel's surface, not every field of it.

### Suppressions name their reason

- No module-level `allow(dead_code)` in `src/`, whatever its reason: a blanket
  over a subtree hides what is really dead beside what only looks dead, such as
  the plugin API's seams
  (`a_module_level_suppression_is_the_one_that_has_to_argue_for_itself`).
- A suppression of a lint about unused surface (`dead_code`, the `unused_*`
  lints) carries a reason: a comment on the line above, or an `expect` with a
  `reason`, which the compiler retires once the lint stops firing
  (`a_suppression_that_hides_unused_surface_says_why`). An `expect` cannot
  serve a registration seam, which is dead in the bin build and live in the
  test build.
- A `clippy::too_many_arguments` suppression needs no sentence and must sit on
  a function (`a_too_many_arguments_suppression_sits_on_a_function`); nothing in
  `src/` suppresses a deprecation (`no_file_in_src_suppresses_a_deprecation`).
  When adding a suppression, name the item and what retires it.
- The registration seams (`capabilities::register`, `kinds::register`,
  `payload_handlers::register`) are called by nothing in-tree by definition, so a
  mechanical dead-code sweep deletes the plugin API; `panel_registry::register`
  has a caller, the panels a plugin that stays loaded adds. A sweep intersects what `cargo check -p openrage --bins` and
  `--tests` report dead, and still misses a field read only through a derived
  `Debug` and a live call graph nobody feeds data into.

Splits the gate does not require are [R13](../direction/Roadmap.md#r13-step-5-shrink-the-shell)'s.

## Gen8 and Gen9: Enhanced is a separate target

Code and documents say Gen8 and Gen9 for a resource's target, with a `G8` or
`G9` suffix on a type; anything a user reads says Legacy and Enhanced, through
`Generation::edition_label()`. An Enhanced install ships Gen8 resources, so a
layout comes from the file's own RSC7 version
(`ResourceType::generation_of_version`), never the install. `Generation`
(`crates/io/or-io/src/generation.rs`) is an open newtype over a target id. The
rules carry into `or-io` ([Gen8 and Gen9](../direction/OR_IO_Specification.md#gen8-and-gen9)).

- A type whose field set Gen9 changed keeps a `G9` type: the vertex buffer, the
  index buffer, the texture and the shader. Every other type both carry is
  described identically.
- A type that differs only in what it points at takes a target parameter,
  `GtaDrawable<Gen9>`, with no `G9` alias.
  A Gen8 type never holds Gen9 data: a struct with per-target field presence
  was built and reverted because it let a Legacy document carry Enhanced data.
- `.ytd`, `.ydr`, `.ydd`, `.yft` and `.ypt` open and save as Enhanced as the
  owned Gen9 roots `or_io::Asset` names (`DrawableGen9` and its siblings),
  with no pack, in the one `DocPayload::Resource` payload
  (`every_enhanced_row_opens_and_saves_with_no_pack`).
- A generic write defaults to Gen8, and the Gen9 writers name their target
  (`gen9_registration_does_not_change_the_write_version`). Two defaults are
  still guesses: `parse_payload` reads a version it cannot place as Gen8, and
  `PayloadRegistry::get` falls back to a format's Gen8 handler.

An Enhanced drawable frames the camera, draws a wireframe
(`an_enhanced_drawable_draws_a_wireframe`) and its mesh through the walk the
Legacy one takes, browses its textures, outlines through
`section_drawable_tree` and inspects through
`src/ui/panels/inspector/gen9.rs`. Every path the viewer runs takes both
targets (`every_viewer_path_that_matches_a_legacy_asset_handles_its_enhanced_twin`):
Material mode finds an Enhanced file's embedded textures, the Animation
workspace offers its meshes and skeletons, and the gallery lists an Enhanced
`.ypt`'s models, the one selected drawing and framing. The panels around the
viewer take both targets too
(`an_enhanced_particle_file_fills_the_outline_as_a_legacy_one_does`): an
Enhanced `.yft` and `.ypt` list their textures, and an Enhanced `.ypt` has its
outline, status-bar summary, Inspector and timeline, each reading the list
through `ParticleTarget` in `src/state/particle_target.rs`. An Enhanced `.ypt`
is not simulated, and the material lookup by name has no Enhanced row, because
an Enhanced shader names its effect by a numeric preset.

⚠ A round trip cannot see a coherent wrong description, because it reads and
writes through the one description: a Gen9 description whose fields move
together stayed green through a round trip, and a swap of two same-sized
fields left every Enhanced `.ydr` opening unless something dereferenced them.
The recorded testimony sees both. The layouts are the local
[Gen9_Resource_Layouts_Specification.md](../extra/Gen9_Resource_Layouts_Specification.md).

## Adding a title

A title costs two things, and `crates/title/or-title/tests/adding_a_title.rs`
proves it with a synthetic title declared and registered inside the test: one
`impl GameProfile`, whose required methods are all facts about the title (the
file stops compiling if the trait gains another), and one registry line, placed
by how specific its install marker is. Detection, executable lookup, mount
layout, DLC policy, support status, store discovery, the keys-cache filename
and the picker's artwork follow from those. No pack is loaded for a title
(OD-OTHER-TITLES, P63, #335): a title on GTA V's target reads GTA V's
declarations, and one with its own target reads nothing until its formats are
declared.

- Registry order is load-bearing: a title with a loose install marker asked
  first claims installs that are not its own, and an Enhanced install ships
  `GTA5.exe`. A registered title is asked after the builtins
  (`registering_a_title_does_not_reorder_detection`).
- `Game` is a handle over a `GameId` string, not an enum; `Game::GtaVLegacy`
  and its siblings are associated constants, and `prefs.json` still loads the
  enum names it once stored.
- `is_supported()` derives from the container family, so a title whose
  container is unimplemented is greyed out with its reason.
- A title, a target and an edition are three things, and a title declares its
  own targets, as RDR2 declares `Generation::from_id("rdr2")`. A title that
  declares `Generation::Gen8` reads "Legacy" wherever a user sees its target.
- A title is not yet a data manifest, because Enhanced names `GTA5.exe` for
  key extraction and refuses it for detection
  (`one_shipped_title_needs_behaviour_and_the_rest_are_data`).

RDR2 is declared, discovered and unsupported until a second container exists;
[R56](../direction/Roadmap.md#r56-rdr2-the-second-container) is its plan.
GTA VI is declared with a target of its own (`Gta6` in
`crates/title/or-title/src/game/profiles.rs`): its install opens as a plain folder and an archive as a
scan, since the archive's table is encrypted
([GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md)).

## The container seam

Where a second container would plug in; RSC7 and RPF7 are the only
implementations, and a joint exists only where a second consumer does.

### The resource envelope

`ResourceEnvelope` (`crates/io/or-io/src/container/resource/envelope.rs`) is
`detect`, `peek`, `open` and `reseal` with an opaque `Seal`; `Rsc7` is the one
implementation. `Opened` carries the payload (the virtual half, then the
physical), both sizes, the version and the seal.

- ⚠ An envelope is defined by the round trip, not the read: whatever the format
  preserves has to survive `open` → `reseal`, so `Rsc7Seal` reproduces its
  header and has no public field-by-field constructor. RBM's byte-identity gate
  is the only thing that would notice otherwise.
- `Rsc7::author` invents a header for a new file; `reseal` reproduces one, and
  only that can be byte-identical. `reseal` refuses a payload whose length the
  seal does not describe. `Rsc7::header_from_entry_flags` rebuilds the
  header an RPF7 entry stripped from the flag words it kept.
- The magic is `pub(super)` in `resource/`, and nothing outside names the old
  header reader and writer. The header is also declared for `or-io` as the
  public `DatResourceFileHeader`.
- DEFLATE stays outside the envelope because RPF7 inflates entries that are not
  resources, and
  the envelope stays usable with no paging context because RBM carries metadata
  in RSC7.

### The paging model

`PagingModel` (`crates/io/or-io/src/desc/image.rs`) is two bases and an offset
mask. Both readers, `DatResource` and `Image`, carry one
(`a_resource_with_other_bases_reads_through_both_readers`), and
`ResourceHeap::with_paging` takes one for writing
(`crates/io/or-io/tests/a_second_paging_model_on_the_write_half.rs`), where
`virtual_address` invents an address and `virtual_offset_of` inverts it. Within
`paging/` only the heap invents an address that reaches a file
(`only_the_heap_invents_an_address_that_reaches_a_file`); `or-io`'s `decl` and
`gta5::formats` still subtract the named base by hand. The test model widens the
mask as well as moving the bases, because code hard-coding the top nibble passes
a test that only moves them. The pointer width is parameterised nowhere.

### The heap and the packer

`ResourceHeap` (`crates/io/or-io/src/container/paging/heap.rs`) appends to a
virtual and a physical heap and records fixups; `PgRscBuilder` adds the RSC7
bucket packer. The canonical writer takes a `ResourceHeap`, so reaching the
packer from it does not compile (`nothing_here_reaches_the_bucket_packer`).
RSC7 stores no fixup table, since the game walks types to rebase pointers; the
fixups are kept for an envelope that does. The write half is
concrete types, not traits: a trait with one implementation is a guess, and
promoting one later is mechanical, while the read seams keep traits because
they have a second consumer.

### The archive and the version registry

`RpfCryptoProvider` is the only trait on that side: a new title brings keys, a
new archive generation brings a layout, and those are different events. A
generation is a value of `RpfVersion` and its table a module under
`container/rpf/layout/`: `classic.rs` for RPF0, RPF2, RPF3 and RPF4, `rpf6.rs`,
`rpf8.rs` and `img.rs` for IMG1 to IMG3. RPF7's header and entry table stay
declared (`FiPackHeader`, `FiPackEntry`); the others are ordinary code, because
a declaration follows the GTA source and these are other titles'. No archive
trait was added: every layout reads into the entries and names RPF7 already
reads into, so a hash that has no text is spelled `hash_XXXXXXXX` and nothing
above the reader sees the difference. A table with no directories (RPF8, the
images) gets a root directory at entry 0.

`RpfArchive::open` and `from_bytes` find the version from the first four
bytes (an IMG1 has none: its `.img` opens beside its `.dir`), and their `_with`
forms take the keys of an encrypted one, which the archive keeps, so no read or
nested open asks for them again. `edit` hands back an `RpfBuilder` holding every
file, laid out and encrypted as it was and read only when the builder writes;
`insert`, `remove` and `rename` change it by path and `save` replaces the file
only once all of it is written, so an archive is saved over itself safely. The
builder keeps paths, not a table: the table is laid out from the sorted paths
at save, so the same files give the same archive whatever order they were added
in, and adding to a path replaces what was there. `insert_stored` carries a body
exactly as its entry stores it, which is how an RPF8 holding Oodle is rebuilt
without decoding it, and the only way a resource before RPF7 is written: its
header is not one this crate can author from a file. `edit` carries every body
as stored in every version but RPF7, and `Defrag` is `edit` with a size preview.
`RpfEntryTag::of` is the one decoder of RPF7's entry
bitfield, so NG key recovery cannot misread it and report every key wrong;
`hide_big_resource_size` and `read_big_resource_size` are the one rule for a
resource too big for the entry's size field. The key code under
`container/rpf/crypto` is title work and moves with nothing else.

A table, or a body, the title enciphers (RPF2 to RPF6 and IMG3 with a header
tag, RPF8 with a key id other than `0xFF`) is `RpfEncryption::Tagged` and is
refused by name until a provider holds that title's key; the GTA V provider
refuses it too, where `Other` would have been taken for an NG mode. An RPF8
table in the clear opens and writes without one.

`VersionRegistry` is keyed by `(TitleId, ResourceType)` and ships one title,
`GTA5` (`the_shipped_registry_holds_exactly_one_title`); a second title adds
rows (`a_second_title_is_a_register_call`), and an openable title must have a
table (`every_openable_title_has_a_version_table`). `ResourceType` is still GTA
V's enum, so a second title's format list is an edit to `or-io`.

### What a second container costs

| Concern | Seam | What it pays |
|---|---|---|
| Paging, read and write | `PagingModel` | nothing, if it keeps RAGE's two bases and the virtual-then-physical payload |
| Page packing | the heap and packer split | its own `allocator.rs` and `chunk_map.rs` |
| Resource envelope | `ResourceEnvelope` | an implementation, and call sites that name `Rsc7` directly |
| Archive | `RpfVersion` | a layout module, and nothing above it: the generations to RPF8 and the images are written |
| Keys | `RpfCryptoProvider` | a provider for its keys |
| Version numbers | the title-keyed registry | a row per format |
| Pointer width | none | a fork of the paging layer |

Whether RSC8 keeps RAGE's bases cannot be settled by reading, and no RSC8 file
exists here.
