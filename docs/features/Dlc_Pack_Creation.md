# DLC pack creation

What a GTA V DLC pack states about itself, how OpenRage writes one, and which
half of "it works" this project can check.

## What a pack is

A pack is `update/x64/dlcpacks/<name>/dlc.rpf` plus a `dlcpacks:/<name>/` line
in `dlclist.xml`, which lives outside the archive. Inside it, `setup2.xml` says
who the pack is (name, device, type, mount order, the change-set groups that
run), `content.xml` what it registers and which change sets enable each file,
and the rest of the tree is the content.

`crates/title/or-title/src/dlc/` reads and writes all three documents, and
`build_pack` writes a `DlcPackSpec` to `dlc.rpf` unless `DlcPackSpec::problems`
objects. `openrage-cli dlc create <src_dir> <out_dir> --name <name>` (plus
`--type`, `--order`, `--minor-order`) packs `<src_dir>` into
`<out_dir>/<name>/dlc.rpf`, unencrypted, and prints the `dlclist.xml` line;
`dlc inspect` reads a pack back. The app's Create DLC pack…, on a folder's
menu, asks what the command asks, shows what it packs and refuses, and builds
through the command's own call, `create_dlc_pack`. Nothing writes into a game
install: installing is copying the folder and adding the line yourself, and
the app refuses a destination inside one.

A project build is a third way to the same pack: an output of the kind `singleplayer-dlc`
writes `<name>/dlc.rpf` from its plan through `build_pack`, takes each file's type from a
`data_file 'TYPE' 'path'` line (the Files content states them as `file_types`) and the two extensions above,
packs a path through a folder named `x.rpf` as that nested archive, and reads the pack and the
archives inside it back against its plan
([what the targets build](../direction/Project_Model_Design.md#what-the-targets-build)).
A content may state what such an archive holds, which the pack writes as the `contents` of its
registration (`PackFile::with_contents`), and may register a name that is no file of the pack
(`PackFile::asked`): the `.ityp` of a `.ytyp` inside a nested archive of that folder, which is how
the install's packs register a type file, and of which it may state the contents too.

## Maps in a pack

A Map content handed to a singleplayer DLC is laid out as the packs of the game lay theirs out,
which `how_the_packs_of_an_install_carry_their_maps` measured on the owner's Legacy and Enhanced
installs (2026-11-25; the figures are in the table below, and
[R70q](../direction/Roadmap.md#r70q-a-build-a-server-and-a-story-install-take) says what is left).

- **The maps and their manifest** go into `x64/levels/gta5/<content>.rpf`, which the pack registers
  as `RPF_FILE` stating `CONTENTS_DLC_MAP_DATA`, disabled and persistent. Every archive of maps of
  both installs is under that folder, and every one a pack registers is stated so.
- **A type file** goes into `x64/levels/gta5/props/<type file>.rpf` with the models and the texture
  dictionaries its archetypes name. The pack registers that archive stating nothing and asks for
  the type file as `DLC_ITYP_REQUEST` stating `CONTENTS_PROPS`, under
  `x64/levels/gta5/props/<type file>.ityp`. No pack of either install asks for a type file that sits
  beside its maps, and each type file a pack asks for is the one type file of such an archive.
- **A model or a texture dictionary no archetype names** goes into
  `x64/levels/gta5/props/<content>.rpf`, registered the same way and asked for by nothing.
- **A type file that declares an interior** is refused by name: no pack asks for one, and the packs
  state it by an entry of the manifest of its archive, which a build does not write. The content
  hands the sentence over as a lock, so the cell of the Builds window says it too and the plan is
  refused by it
  (`a_type_file_that_declares_an_interior_is_asked_for_by_nothing_and_refused_for_a_pack_by_name`,
  `what_a_content_says_locks_the_build_it_is_laid_out_for_locks_the_cell_and_stops_the_plan`).

The pack is written, read back and counted by the code the install suite runs
(`a_story_pack_keeps_its_maps_and_its_type_files_in_the_archives_the_packs_of_the_game_keep_theirs_in`,
`a_story_pack_reads_back_as_the_packs_of_an_install_read_but_for_what_turns_its_maps_on`). A content
of type files and models plans and builds, and its cell in the Builds window says yes
(`a_content_of_type_files_and_models_plans_and_builds_as_a_singleplayer_dlc`). Which archive a model
or a texture dictionary goes into, by its asset, a dictionary of drawables or a texture dictionary
an archetype names, is held by
`a_model_and_a_texture_dictionary_go_into_the_archive_of_the_type_file_whose_archetype_names_them`.

What the packs of the two installs state, as `how_the_packs_of_an_install_carry_their_maps`
counted it on 2026-11-25:

| What | Legacy | Enhanced |
|---|---|---|
| Packs mounted beside the title update, and those that hold a map file | 86, 34 | 103, 37 |
| Archives that hold a map file, every one under `x64/levels/gta5` | 161 | 164 |
| Of them in that folder directly, one folder below, two below | 10, 46, 105 | 10, 48, 106 |
| Registered by their pack, each `RPF_FILE` stating `CONTENTS_DLC_MAP_DATA`, disabled and persistent | 142 | 146 |
| Of those, registered as an overlay | 44 | 44 |
| Not registered, and of those at a path the base game holds an archive at too | 19, 17 | 18, 17 |
| Holding a type file, a manifest, a model or a texture dictionary, a bound | 90, 112, 10, 10 | 94, 113, 10, 9 |
| Turned on by `GROUP_MAP` through the list of a map, each in a compat pack | 99 | 103 |
| Turned on by `GROUP_UPDATE_STREAMING`, each in a level pack | 34 | 34 |
| Turned on by a change set no group names | 9 | 9 |
| Turned on by `GROUP_STARTUP` or by `GROUP_MAP_SP` | 0 | 0 |
| Registrations stating map data of an archive that holds no map file | 52 | 94 |
| Manifests in the packs, every one `_manifest.ymf` at the top of a nested archive | 906 | 970 |
| Type files in the packs, and those beside a map file | 1,814, 1,134 | 1,943, 1,248 |
| Type files beside a map file that a registration asks for | 0 | 0 |
| Type files beside a map file whose archive holds a manifest, and those it names as needed | 692, 687 | 695, 692 |
| Type files a registration asks for, each as `DLC_ITYP_REQUEST` stating `CONTENTS_PROPS`, the one type file of an archive that holds a model, no map and is registered with no contents | 525 | 530 |
| Of those, under `x64/levels/gta5/props` | 517 | 522 |
| Requests that are not persistent, and persistent | 476, 49 | 481, 49 |
| Archives of a request, each registered disabled and persistent, and those of them registered as an overlay | 525, 8 | 530, 8 |
| Request and archive turned on by `GROUP_STARTUP`, by `GROUP_UPDATE_STREAMING`, by no group | 510, 14, 1 | 515, 14, 1 |
| Type files asked for that a manifest names as needed too | 227 | 228 |
| Type files that declare an interior, none of them asked for | 264 | 279 |
| Of those in an archive with a manifest, and those it has an entry for | 212, 212 | 216, 216 |

Where the packs differ among themselves a build takes the larger share: a request is not
persistent, and the archive of maps is no overlay. That archive sits in the level's folder
directly, as ten archives of each install do, where the others sit one or two folders below; and a
manifest of a build's maps may name a type file the pack also asks for, as the manifests of the
game do for some and not for others.

⚠ **A content with a map is still locked, by one sentence.** A pack built here turns every file on
from one change set of `GROUP_STARTUP`. No pack of either install turns an archive of maps on from
that group, and no setup states `GROUP_MAP_SP`: the packs turn theirs on from `GROUP_MAP`, through
the list of a map, from `GROUP_UPDATE_STREAMING` in a level pack, or in a few cases from a change
set no group names. Which change set a story pack
states for its maps is therefore something no pack shows, and the cell stays locked until a pack
that turns them on at start is seen to load in the game
(`the_cell_of_a_singleplayer_dlc_says_yes_to_type_files_and_models_and_no_to_a_map_for_what_turns_it_on`).

## A clothing pack in a pack

A clothing content ships in a singleplayer DLC and in an `.oiv` as the game's own freemode packs are laid out. Measured over the retail Legacy pack `mp2023_01` (a read-only look at the install, at the revision of the wave that built it): the pack registers four archives as `RPF_FILE`, one for the components of each ped and one for its props (`x64/models/cdimages/mp2023_01_female.rpf`, `..._female_p.rpf`, `..._male.rpf`, `..._male_p.rpf`), the shop file of each ped as `SHOP_PED_APPAREL_META_FILE` in `common/data/`, and the creature metadata as an archive of its own, `x64/anim/creaturemetadata.rpf`. Inside the components archive a folder named for the collection (the female ped's of that pack) holds the models and textures named without the collection (`accs_000_u.ydd`, `accs_diff_000_a_uni.ytd`), the variation metadata is the one `.ymt` beside that folder, and the props archive holds a folder of the same kind for the props. The Wardrobe writes that layout: the file names and folders are `or_wardrobe::layout::Paths`, the shop files are the data files the pack registers, and no `.ymt` is registered, because the retail `content.xml` registers none.

- **Flat.** A pack has no folder for each slot; the option that asks for one is a FiveM resource's.
- **A slot past the cap is several collections.** Each is a folder beside its own `.ymt` in the archive, registers its own shop file as `SHOP_PED_APPAREL_META_FILE`, and has its own creature metadata entry (`a_split_pack_ships_in_a_singleplayer_dlc_with_a_folder_and_a_shop_file_for_each_collection`). A pack whose files replace the game's own is not carried: a pack adds collections, so the plan refuses it with a sentence that names the collection and says a replacement ships in a FiveM resource.
- **A clothes dump is not carried.** It is for the scripts of a server, and a pack registers only files the game reads, so a build of a pack that asks for one says in a note that it left it out.
- **An archive holds a limited number of entries.** The plan counts what each archive would hold (a file, a nested archive and a folder are one entry each, and an archive has a root), keeps the fullest as the budget `archive_entries` and refuses a pack over the 65,535 that the clothing tools state an archive holds, or over a lower ceiling the build sets (`seventy_thousand_entries_in_one_archive_are_refused_with_the_sentence_that_says_why`). A build that divides cuts between contents, so two clothing contents of 35,000 files each are two packs (`seventy_thousand_entries_divide_into_two_packs_at_the_boundary_between_contents`); one content over the figure by itself is refused with a sentence, and cutting inside one is wave P29's.
- **An `.oiv` installs the same pack** below `update/x64/dlcpacks/<name>/dlc.rpf`, with the `setup2.xml` and `content.xml` that make it a pack, and lists every file in its manifest. It edits no `dlclist.xml`, which OpenIV's add command cannot do, so the plan carries a note with the line to add by hand.
- **A built pack reopens as a project.** `openrage-cli wardrobe reopen <dlc.rpf or file.oiv> --out <folder>` reads the files of the pack back, archive by archive, and makes a project of sources with the garments the metadata states; every file it cannot place is named and kept under `files/` ([what reopening and exchange build](../direction/Clothing_Tool_Design.md#what-the-engine-builds)). It reads files, and says nothing of whether the game mounts the pack.

## Traps

⚠ **A pack can mount cleanly and do nothing.** Every `dataFiles` entry in a
shipped pack is `disabled="true"`: a `contentChangeSet` naming the file in
`filesToEnable` enables it, and a `contentChangeSetGroup` in `setup2.xml`
(`GROUP_STARTUP`, `GROUP_MAP`) runs that change set. `DlcPackSpec` writes every
file disabled, one `<NAME>_AUTOGEN` change set enabling them all, and a
`GROUP_STARTUP` group running it.

- **A map change set runs in one mode.** A change set a setup names under
  `GROUP_MAP_SP` and no other group is the story's, one under `GROUP_MAP` alone
  is multiplayer's, and `GROUP_STARTUP` runs in both. A mount that is given a
  mode (`MountOptions::mode`, `openrage-cli dlc-list --mode`) enables the files
  of the change sets that apply and names the ones it left out; a change set no
  group states a mode for applies as it does with no mode, and is named too
  (`a_change_set_stated_for_multiplayer_applies_there_and_not_in_story_and_the_reverse`).
  No document of a pack says that `GROUP_MAP` is multiplayer's: its name says
  only that it is the map group that is not the story's. On the owner's two
  installs every pack that names a change set under `GROUP_MAP` is a compat
  pack, all but one of them named as a multiplayer pack, and no setup states
  `GROUP_MAP_SP`, so the rule leaves out of the story what `GROUP_MAP` names
  and nothing out of multiplayer
  (`which_groups_the_change_sets_of_an_install_belong_to`, 2026-11-25, each
  pair Legacy and then Enhanced). Of the 389 and 432 change sets the setups of
  every pack and of the title update state, `GROUP_STARTUP` names 100 and 124
  and `GROUP_MAP` names 82 and 86, in 31 and 34 packs that are all compat
  packs, 30 and 33 of them named as multiplayer packs. The rule leaves those 82
  and 86 change sets out of the story and none out of multiplayer, and names
  207 and 224 as stated for no mode, which are under the nine other group names
  the setups state or under none. No document says in words that either map
  group is one mode's: that stays the mount's reading of the names, bounded and
  not proven, held on a setup shaped as the installs state them by
  `of_the_groups_an_install_states_the_story_leaves_out_the_multiplayer_map_group_alone`.
- **The device follows `minorOrder`.** `content.xml` paths use `dlc_<name>CRC:`
  when `minorOrder` is 0 or less and `dlc_<name>:` otherwise, and a pack using
  the other mounts with paths that never resolve (`SetupData::content_device`).
- **An extension settles `fileType` for `.ityp` and `.rpf` only.** Over the 184
  packs of two retail installs, `.ityp` is `DLC_ITYP_REQUEST` 1,054 of 1,054
  times and `.rpf` is `RPF_FILE` 5,951 of 6,015, while `.meta` (1,898 entries,
  most often `WEAPONINFO_FILE`, 280) and `.xml` (328, most often
  `OVERLAY_INFO_FILE`, 68) have no majority (`3e8a507f`). So `file_type_for`
  answers for those two, and `DlcPackSpec::problems` refuses any other file
  until its author states a type, because the game ignores a registration of a
  type it does not expect. The CLI cannot state one, so it refuses a source tree
  holding anything but `.rpf` and `.ityp` files; code uses
  `PackFile::with_file_type`.
- **Any `<Item>` may carry a `platform`** (`ps4|xboxone`, `ps5|xbsx`): 256 do in
  `dlclist.xml`, `filesToEnable` and `mapChangeSetData` across two retail
  installs (`3e8a507f`). The model keeps it, since a pack edited on PC and
  written back unscoped would enable console-only files everywhere.

## What is verified

Nobody working on this repository can run GTA V, so
[R51c](../direction/Roadmap.md#r51c-dlc-pack-creation)'s "loads in game with no manual
editing" stays open: nothing here says the engine mounts a pack, fires its
change sets or honours its mount order. What is checked:

| Claim | Test |
|---|---|
| Every DLC document two retail installs ship parses, writes and parses back to the same model; a second write changes nothing; no `<Item>` value is lost against an independent XML walk; `file_type_for`'s rulings are the majority in both directions | `crates/title/or-title/tests/what_a_shipped_dlc_pack_survives.rs` |
| A built pack reads back as the archive its spec described | `a_built_pack_reads_back_as_the_pack_that_was_specified`, `the_archive_would_notice_a_file_that_was_never_added` |
| A spec that would not load is refused before anything is written | `a_pack_that_would_not_load_is_refused_before_anything_is_written` |
| A build told to stop leaves no archive, and one told to go on writes what `build_pack` writes | `a_build_told_to_stop_leaves_no_archive_behind` |
| A folder built from the app is the command's pack, byte for byte | `a_folder_packs_from_its_menu_into_the_pack_the_command_builds_of_it` |
| Every registered file is enabled by the pack's own change set, against the device the mount registers | `a_generated_pack_enables_every_file_it_registers`, `a_generated_pack_writes_its_paths_against_the_device_the_mount_registers` |
| An escaped value comes back as itself | `every_escaped_value_comes_back_as_itself` |
| A project output writes a pack that reads back as its plan, with each file registered under the type it states, and the same pack twice | `a_dlc_output_writes_a_pack_that_reads_back_and_registers_each_file_with_its_type`, `two_builds_of_a_dlc_output_are_byte_identical` |
| A path through an archive's name is packed as that nested archive, and the pack registers the archive alone | `a_path_through_an_archives_name_is_packed_as_that_nested_archive` |
| A type file inside a nested archive is asked for by the `.ityp` name of the archive's folder and by no other, the pack states props of that name, and an archive is said to hold something only when the plan puts a file in it | `a_type_file_in_a_nested_archive_is_asked_for_by_the_name_of_its_folder_and_by_no_other`, `an_archive_said_to_hold_something_is_one_the_plan_puts_a_file_in` |
| A pack built from a Map content keeps the pattern the packs of an install keep, in everything but the change set that turns its maps on | `a_story_pack_reads_back_as_the_packs_of_an_install_read_but_for_what_turns_its_maps_on` |
| Two contents that say one archive holds different things are named, a map file that would sit loose is refused once and not also as a file with no type, and a name a pack asks a type file by is registered only while that type file ships | `two_contents_that_say_one_archive_holds_different_things_are_named`, `a_map_file_that_would_sit_loose_is_refused_once_and_the_line_that_declares_a_map_is_passed_by`, `a_name_a_pack_asks_a_type_file_by_goes_with_that_type_file_and_stays_with_another` |
| A file with no settled type, a type for a file the project lacks, a script and a name that is no device name are refused at planning | `a_file_whose_extension_settles_no_type_is_refused_at_planning_by_name`, `a_type_stated_for_a_file_the_project_does_not_hold_is_a_problem`, `a_script_cannot_ship_in_a_dlc`, `a_name_that_is_not_a_device_name_is_refused_at_planning` |
| A pack that is not its plan is found, whatever it lacks | `verification_notices_a_pack_that_is_not_the_plan` |
| A clothing pack is laid out as the game's own packs are, registers the shop files and each archive, reopens through the archives, and is the same bytes twice | `a_pack_is_laid_out_as_the_games_packs_are_and_registers_what_the_game_is_told_of`, `a_dlc_rpf_reopens_through_the_readers_and_the_clothing_engine_checks_what_was_read` |
| An `.oiv` lists the files of the pack it installs and reads back through the clothing check | `an_oiv_lists_the_files_of_the_pack_it_installs_and_reads_back_through_the_clothing_check`, `the_files_of_a_pack_are_installed_inside_the_dlc_rpf_with_the_documents_that_register_them` |
| An archive of more than 65,535 entries is counted, refused, and divided between contents | `seventy_thousand_entries_in_one_archive_are_refused_with_the_sentence_that_says_why`, `seventy_thousand_entries_divide_into_two_packs_at_the_boundary_between_contents` |
| A build can be stopped while it writes, and removes what it was writing | `a_build_stopped_while_it_writes_leaves_the_last_good_build_and_no_half_written_folder` |

The unit tests are `crates/title/or-title/src/dlc/tests.rs`, and the project output's are `crates/domain/or-project/tests/a_singleplayer_dlc_builds_from_a_plan.rs`. The install suite
(`what_a_shipped_dlc_pack_survives`) needs `OPENRAGE_GAME_ROOT` (roots separated by `;`); the NG keys every retail
archive needs come from the bundled key snapshot unless `OPENRAGE_KEY_SNAPSHOT`
names another. With no root it skips,
and a skip is not a pass. It fails rather than pass on less: on an unreadable
pack, fewer than 20 packs, 10,000 elements or fewer, or no change set, map
change set, group or `contents` element exercised. The one shape it lets
shrink is a second `<dataFiles>` section in one shipped pack, whose items all
survive in the single section written back (`MERGED`).
