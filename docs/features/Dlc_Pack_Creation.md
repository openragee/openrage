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
  only that it is the map group that is not the story's. No install was at hand
  to count the packs that use either group.
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
