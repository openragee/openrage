# GTA VI platforms plan

How OpenRage's GTA VI support stops being support for one Xbox package and becomes support for a title that ships on several platforms: what a PlayStation 5 build holds, where the tree bakes in Xbox facts, the design that makes the platform an explicit axis, and the order of the rows that build it.

Status: the plan of 2026-11-28; its rows R72a to R72e are built, and its decisions are the plan's own until the owner rules on [what is asked](#asked-of-the-owner). [R72](Roadmap.md#r72-gta-vi-on-every-platform) says where each row stands.

## The shape in brief

GTA VI was built against one build, and the tree cannot tell a fact of the game from a fact of the Xbox. A PlayStation 5 build arrived on 2026-11-28 and nothing of it opens. Five decisions shape the rework:

1. **A file's platform is read from the file, never from the install.** An RPF9 header and a texture pack's directory each state the platform letter. This is the rule a target already follows: a file's target is its RSC7 version, never the install it came from.
2. **A platform fact is only what no file states.** One small table in `or-io` holds them. What a file states of itself (a pack's envelope by its magic, a chunk's coder by its row, an executable's form by its head) is read from the file and is in no table.
3. **An unmeasured platform is refused by name.** Nothing is written for PC now. Each seam matches on the platform, and a platform with no measured row is an error that names it, so a PC build later is a row of the table and a profile.
4. **A profile is a build, and a title is what groups builds.** PlayStation 5 is one more profile and one registry line, as `crates/title/or-title/tests/adding_a_title.rs` already holds for a console. The title becomes an id code can test, where today it is a label.
5. **Keys are held per build and handed to an open.** The process-wide context goes. An archive whose build has no keys held opens by scan and says which build's keys are missing.

**This plan produces no key.** The tables of the PlayStation 5 build are enciphered and the key snapshot held for the Xbox build does not open them. Where a PlayStation 5 snapshot comes from is outside this plan: the design gives it a place to be handed in, and the rows that need the tables read are marked as waiting on one.

## What it rests on

- **The tree at `6b1dda52`**, read on 2026-11-28 by three reviews: `or-io`, `or-title`, and the app with the CLI and the shader crates.
- **The two builds**, measured on 2026-11-28 with `openrage-cli` of that revision and byte reads: the Xbox Series package 1.0.2.1 and the PlayStation 5 package of content version 01.002.001. The byte-level record is the local [PlayStation 5 notes](../extra/gta6/ps5/README.md).
- **The rules of [AGENTS.md](../../AGENTS.md)**: layers run one way, a reader refuses bad bytes with an error, and a check fails once before it is trusted.
- **[R36](Roadmap.md#r36-console-and-platform-variants)**, which declined console variants until fixtures existed. A PlayStation 5 build is that fixture for GTA VI, and for no other title.

## What the PlayStation 5 build is

Measured on 2026-11-28. A cell is what the build's own bytes state.

| | Xbox Series | PlayStation 5 |
|---|---|---|
| What marks the title | `MicrosoftGame.Config` and its title id | `sce_sys/param.json` and its title id |
| Executable | a PE, `game_scarlett_master.exe` | `eboot.bin`, a SELF container around an ELF |
| Archives | one `X.rpf` each | `X.rpf`, and for most a `X_concat.rpf` beside it |
| RPF9 header, the platform letter at 0x19 | `z` | `p` |
| RPF9 header, the byte at 0x2E | 9 | 7 |
| Bodies | zlib chunks | stored as they are, from any 16-byte slot |
| Texture pack | an `RDS5` row table whose row 0 is the `ETPF` directory | the `ETPF` directory at byte 0, then the texels |
| Texels of one texture | the Xbox tiles | the same image, in its own order inside a 64 KiB tile |
| Tables | enciphered | enciphered, and not opened by the Xbox snapshot |

- **Both files of a pair are whole archives.** A `_concat` archive has its own header and table, and its content is nested archives. Whether a row of `X.rpf` points into `X_concat.rpf` or the two mount side by side is [not measured](#measurements-still-owed).
- **A PlayStation 5 pack is an image stored plainly.** The Xbox rows decode into an image at virtual addresses; a PlayStation 5 file is such an image as it is, behind the same directory, each entry equal to the Xbox one but for its page. A range there is padded, so a texture's length comes from its entry. The Xbox coder and its tables play no part on PlayStation 5.
- **The texel order differs only inside a 64 KiB tile.** A texture both builds hold is the same bytes on both, except in the tiles of tile mode 0, where a PlayStation 5 tile is the 4 KiB tile's order and then the Xbox tile's other bits. The Xbox reader was the oracle, and `a_texture_two_platforms_hold_reads_to_the_same_pixels_through_each_envelope` holds the two readers to each other. ⚠ A shared key is not proof of a shared picture: the pack of controller art holds another drawing under one key on each platform.
- **The keyless scan finds nothing.** Its detectors look for zlib chunk heads. On PlayStation 5 a body starts on a 16-byte slot with its own first bytes in the clear: a nested archive, an effect container, a movie, a metadata body. Bodies are packed back to back or set on a lattice of their own archive, so no grid finds them.

## What the tree does with it today

Run on 2026-11-28 at `6b1dda52`.

| Step | Result |
|---|---|
| Add the folder as a game | "No game OpenRage knows"; `Game::detect_from_root` answers GTA V Legacy for it |
| `rpf9 list`, any archive | refused: the byte at 0x2E is not 9 |
| The same with the Xbox snapshot and that byte forced | refused: the table does not decipher |
| `rpf scan` | refused at the header; forced, it finds no body |
| `ttpf info` | refused: the file does not start with `RDS5` |

## Where the tree holds Xbox facts

Read at `6b1dda52`. Each is a fact of one build written as a fact of the game.

**The archive, in `or-io`.**

- `Rpf9Header::broken_rule` in `crates/io/or-io/src/container/rpf/rpf9.rs` holds one table of constants, the Xbox's, and every other rule there was measured on Xbox headers alone. `crates/io/or-io/src/container/rpf/gta6_container_build.rs` writes the Xbox letter and byte into every archive it builds.
- The platform letter is read and nothing in `or-io` decides by it; the app shows it in a row's extension, and falls back to the Xbox's where no table opened. The list of letters exists twice in `or-io` (`RAGE_PLATFORM_LETTERS` and a copy in the header's rules) and a third time as `Platform::extension_letter` in `or-title`.
- RPF9 is no layout behind `RpfArchive`: it is a side state and a branch in each of `crates/io/or-io/src/container/rpf/reader.rs` and `crates/io/or-io/src/container/rpf/archive.rs` where the version is RPF9.
- A table's keys come from process state. `Gta6Context` in `crates/io/or-io/src/container/rpf/gta6_rpf.rs` is a global that reads environment variables once and keeps the result, errors included; the provider an archive is opened with is ignored for RPF9. The app keeps one key set for the process in `src/state/rpf9_table.rs` and gives it to that global, so two builds cannot be open at once, and the CLI loads keys through two functions that do the same.
- The key code names one build. `crates/io/or-io/src/container/rpf/gta6_extract.rs` lists a single build, reads it as a PE in one of two PE readers the tree holds, and keeps one cache file, `gtavi_keys_cache.dat`, for every install; `GtaViKeys` records no platform, and a snapshot is used without holding its recorded executable to the build it is used on.
- `crates/io/or-io/src/container/rpf/rpf9_scan.rs` is built on zlib heads from end to end, and the readers of `crates/io/or-io/src/container/rpf/gta6_mount.rs` and `crates/io/or-io/src/container/rpf/gta6_chunk_table.rs` assume zlib where a chunk is not stored.

**Textures.**

- `crates/io/or-io/src/gta6/ttpf.rs` takes `RDS5` as the pack's magic and the directory as its row 0, so the envelope and the pack are one type.
- `crates/io/or-io/src/gta6/tiling.rs` is the Xbox tile order with no name that says so, and `crates/io/or-io/src/gta6/headerless/texture.rs` refuses any tile mode but the one measured.

**Titles and installs, in `or-title`.**

- `Gta6::platform` answers Xbox Series for every folder, and the profile names one Xbox executable and no PlayStation title id, though `playstation_title_ids` exists and a test profile uses it.
- `Game::detect_from_root` falls back to GTA V Legacy when no profile matches, so an unknown folder is treated as GTA V by every caller.
- `InstallLayout` is read by nothing but a test: the GTA V mount hard-codes its platform folder, and `platform_archive_prefix` goes unused.
- A title is a label. `GameProfile::title` returns a string the game picker groups by, and code that means "any GTA VI build" compares with `Game::GtaVI`.

**Walking an install.**

- Six functions list a GTA VI install's archives, each its own walk of the disk: `crates/domain/or-world-gta6/src/index_mount.rs`, `src/state/gta6_install_scan.rs`, `crates/tools/or-cli/src/rpf9.rs`, `crates/tools/or-cli/src/gta6_install.rs`, `crates/tools/or-cli/src/gta6_hashes.rs` and `crates/tools/or-cli/src/gta6_names_mine.rs`. Two more list its packs. None goes through `crates/title/or-title/src/install/crawl.rs`, and none knows a `_concat` file, so each would list every pair as two unrelated archives.
- The executable is looked for in four places, three of them by the `.exe` ending, and the app reads `common.rpf`, `levels.rpf`, `textures.rpf` and `shaders.rpf` by name, each as one file.

**Shaders.**

- The Xbox package's reader is `crates/io/or-io/src/gta5/shaders/xdat.rs`, under GTA V's folder. The native front end's decoder and control-flow graph take plain words (`crates/tools/or-shader-front/src/rdna/decode.rs`, `crates/tools/or-shader-front/src/rdna/cfg.rs`), but its lowering takes a program record filled from offsets of that package, and `crates/tools/or-shader-front/src/rdna_bind/mod.rs` reads the package's binding list at fixed offsets.
- The renderer hands a translated program what the console hands it: image descriptors in `src/render/gta6_meshlets.rs` and the ray tracer's node layout in `crates/runtime/or-renderer/src/game_shaders/trace/arena.rs`. Whether another platform's programs read the same is not measured.

**Tests.** The install-gated suites take one build from `OPENRAGE_GTA6_BUILD`, each reading the variable itself, list every `.rpf` under it, and several assert figures measured on the Xbox build. The pack suite alone goes through `or_test_support::game_roots`, which already takes a list of folders.

## The design

### Title, build, platform, target, install

| Word | What it is | Where it comes from |
|---|---|---|
| Title | the game: GTA V, Red Dead Redemption 2, GTA VI | a profile states it |
| Build | a title as it ships for one platform or edition | one `GameProfile` |
| Platform | the machine a file or a build was made for | a file's own header; a profile for its install |
| Target | the layout of a resource | the file's own version, as today |
| Install | a folder that holds a build | detection, by the build's markers |

⚠ A platform is not an install, as a generation is not an edition. A reader takes the platform from the file it reads, so an archive opened by itself with no game configured reads as it would from its install, and an install never overrides what a file states.

### The platform in or-io

`Platform` moves down from `or-title` into `or-io`, which re-exports nothing upward: `or-title` imports it. It owns the one list of RAGE's letters, and the two copies in `or-io` go.

```rust
/// What no GTA VI file states of itself, for each platform measured.
pub struct PlatformFacts {
    pub platform: Platform,
    /// The header byte at 0x2E, whose role is not known.
    pub header_word: u8,
    pub texel_order: TexelOrder,   // XboxTiles, Prospero
    pub scan: ScanStrategy,        // ZlibHeads, StoredBodies
}

impl PlatformFacts {
    /// The facts of `platform`, or an error naming a platform nothing was measured on.
    pub fn of(platform: Platform) -> Result<&'static Self>;
}
```

The table starts with two rows. A fact joins it only when a measurement shows the platforms differ and no file states the difference, which keeps it from becoming a second configuration system.

### The archive

- **The header's rules split in two.** The rules every RPF9 header holds stay as they are, and the constants that differ come from the facts of the letter the header states. A letter with a byte of another platform is refused, naming both.
- **RPF9 becomes a backend.** `RpfArchive` holds one of two backends, a flat table read by a layout or the RPF9 state, and the branches on the version collapse into that one match. RPF9 is not forced into the flat layout trait, whose every question assumes a table of rows it does not have.
- **Keys arrive with the open.** The provider `RpfArchive::open_with` already takes carries the GTA VI session, the keyring and the names, and `Gta6Context` with its global and its once-read environment is deleted. The CLI and the app build one session and pass it; a test builds its own.
- **A body is read as its row states.** A chunk's coder is in its row, so stored and zlib are told apart there and not by platform, and a coder no reader holds is refused by its number.
- **A written archive keeps the platform it was read from.** A rebuilt or patched archive takes its letter and byte from its source, and `rpf9 build` from a folder takes the platform as an argument. Building for a platform whose alignment is not measured is refused.

Whether a table and its bodies can sit in two files is decided by [a measurement](#measurements-still-owed), not here. `Rpf9Place` already names a file for each body, so the type that would carry it exists.

### The scan

A scan is a strategy chosen by the header's platform. The zlib strategy is today's code. The stored strategy shares its walk over the 16-byte slots and names a body by its first bytes: a metadata document, whose frames state where it ends, an RBF or an XML document, and a headerless resource behind its zero slot. Both give the same rows, so every view and command over a scan is untouched.

⚠ A stored body has an end only where its bytes state one. A metadata document, a movie, an audio container and a stored resource with its head do; a headerless resource does not, so a scan lists it and cannot open it.

### Texture packs

The pack splits along what is shared and what is not:

| Part | Shared | What it is |
|---|---|---|
| Directory | yes | `ETPF`: entries, keys, the platform letter |
| Envelope | no, stated by the magic | `RDS5` rows that decode into an image, or none: the file is the image |
| Coder | Xbox envelope only | BCPack and its tables, under the rows that use it |
| Texel order | no, a platform fact | from a texture's place in the image to its blocks |

`Pack::open` reads the magic, opens the envelope it names and hands every reader above it one thing: bytes of the image at an address. The directory reader, the texture decode, the Gallery, the key index and the name rules then serve both platforms unchanged. `crates/io/or-io/src/gta6/tiling.rs` becomes the Xbox order under its name, beside the PlayStation 5 order, and the texture reader of a headerless dictionary asks the same module with the platform of the archive it came from.

A pack saves in the envelope it was read in. The plain envelope's writer is the image written out.

### Resource readers

No context is threaded through the readers on speculation. A headerless reader takes the platform only where a measurement shows the platforms differ, and textures are the one such place known. The loose resource form is OpenRage's own, so it states its platform in its head from the row that needs it on.

### Keys

- **A key set belongs to a build.** `GtaViKeys` records the platform it was taken for, and a keyring holds one set per platform. An archive asks the keyring with its header's letter.
- **A cache per build.** The one cache file becomes one per platform, and a snapshot is held to the executable it records before it is used.
- **No keys is a state, not an error.** An archive of a platform the keyring has no set for opens by scan, and the message names the platform, where today the table fails to decipher with a sentence about a descriptor.
- **One executable reader.** The two PE readers become one module that maps an address to a file offset and is told the container by the file's head. The Xbox extractor keeps reading through it. Nothing in this plan reads keys from another platform's executable.

### Titles, builds and installs

- **The title is an id.** A profile states its title's id beside its label, `Game::title_id` returns it, and code that means any build of GTA VI tests the title. `Game::GtaVI` equality stays only where the Xbox build is meant.
- **PlayStation 5 is a profile.** It states its title ids and `Platform::Ps5`, and is registered beside the Xbox build. ⚠ It names no executable: every PlayStation package holds an `eboot.bin`, and a folder that holds a profile's executable is claimed by it. The Xbox build keeps the id it had as the only build, because what OpenRage keeps for a build is kept under its id; whether it is renamed so that the plain id is PC's is [asked](#asked-of-the-owner).
- **No silent fallback.** `Game::detect_from_root` answers nothing for a folder no profile claims, and each caller says what it does then.
- **Dead layout goes.** `InstallLayout` is either read by the mount or deleted; the plan deletes it and leaves GTA V's mount as it is.

### One list of an install's files

`or-title` gains the one function that lists a GTA VI install: its archives, each with its role (alone, the first of a pair, a `_concat`) and its pack folder, and its texture packs. The six archive walks and the two pack walks are deleted for it, and the modules that read an archive by its file name ask it for the archive of that role. The executable is found by the profile, in one place.

### The app and the CLI

- A row's extension letter is its archive header's on every path. A scan reads the same clear header a table does, so `platform_letter_for` no longer falls back to a game for RPF9.
- The app holds a session for each build it has open, found by the platform an archive states, so an Xbox archive and a PlayStation 5 archive open side by side.
- The game picker and Settings list both builds under the one title, each with its platform, as they already do for a console.
- The CLI's two key loaders become one helper that builds the session from `--keys`, `--exe` and `--names`, and every `rpf9`, `rpf`, `ttpf` and `hashes` command opens through it.
- The calibration and tables screens of the Xbox coder show only for a pack whose envelope has that coder, and their sentences name the coder where they say "the console's".

### Shaders

The native front end separates along the line its code already has: unwrapping a platform's package into a program record, and decoding, lowering and binding from that record. The Xbox package is the first unwrapper, and its reader moves out of GTA V's folder. The binder asks the unwrapper for the binding list where it reads fixed offsets today. What a PlayStation 5 program's package is, whether its instructions are the ones the decoder reads, and whether its programs read the descriptors and the node layout the renderer hands the Xbox's, are [measurements](#measurements-still-owed). A PC build is expected to go through the bitcode front end the tree already has, which the CLI's one routing point already tries first. Each is refused by name until it is measured.

### Tests over more than one build

- The install-gated suites take a list of builds, each with its platform, from the helper beside `or_test_support::game_roots`, and run once for each. No suite reads the variable itself, and a skip names the build it lacks.
- A figure measured on one build is asserted for that build or becomes a rule every build holds.
- **The two builds check each other.** A texture both hold must decode to the same pixels through two envelopes and two texel orders. That is a comparison with something independent, which no round trip gives.

### The module layout

The archive code is the files named `gta6_` and `rpf9_` in one folder that also holds every other archive generation, several joined by `#[path]`. It moves, last and mechanically, into one folder:

```
container/rpf/rpf9/
  header     keys       toc        table      chunks
  content    mount      names      kinds      session
  scan/      zlib, aligned
  write/     build, rebuild, patch, loose, grow
```

Three duplications go on the way: the column locator written three times, the snapshot constants written twice, and the chunk readers, of which the mount, the patch and the scan each have one.

### PC

Nothing is built for PC. When a PC build exists it costs a profile, a row of the facts table, the envelope and texel order its packs state, its keys handed to the keyring, and the bitcode front end joined to the renderer. Every seam above refuses it by name until then.

## The order of work

Each row ends on a check that fails today. Rows a to e need no key.

| Row | What | Done when |
|---|---|---|
| R72a | `Platform` in `or-io`, the facts table, the header's rules by platform | every header of both builds parses, and a letter with the other platform's byte is refused by name |
| R72b | The title as an id, the PlayStation 5 profile, no fallback in detection | a folder with a PlayStation 5 `param.json` is that build, and a folder no profile claims is no game |
| R72c | One list of an install's files | the walks are deleted, and a pair lists as a pair in the app and the CLI |
| R72d | Texture packs: directory, envelope, texel order | every PlayStation 5 pack reads, and a texture both builds hold decodes to the same pixels |
| R72e | The scan of stored bodies | a scan of a PlayStation 5 archive lists its nested archives and its clear bodies |
| R72f | Keys per build, the session handed to an open, the global deleted | an archive of a build with no keys held opens by scan and names the build; no `or-io` reader reads the environment |
| R72g | RPF9 as a backend of `RpfArchive` | the version branches are one match, with every archive suite unchanged |
| R72h | The tables of a PlayStation 5 archive: rows, pairs, the mount, a census of every reader | waits on a snapshot; then each reader's share read on PlayStation 5 is measured against the Xbox's |
| R72i | Shaders and audio on PlayStation 5 | waits on R72h's measurements |
| R72j | The module move, the suites over a list of builds, the documents | no file named for the Xbox holds a fact of the game, and [GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md) is one document of every build |

R72a stands on nothing and R72b beside it. R72c needs R72b, R72d and R72e need R72a, R72f and R72g follow in either order, and R72j is last because it moves files every other row edits.

Rows a to e are built. What holds each: `a_header_that_mixes_two_platforms_or_states_an_unmeasured_one_is_refused_by_name` and, over the builds `OPENRAGE_GAME_ROOT` names, `every_archive_header_of_a_build_parses_and_states_its_build_s_one_platform`; the cases of `crates/title/or-title/tests/a_gta_vi_folder_is_not_gta_v.rs`; the tests of `crates/title/or-title/src/install/gta6_files.rs`; `a_texture_two_platforms_hold_reads_to_the_same_pixels_through_each_envelope`, which is install-gated; and the tests of `crates/io/or-io/src/container/rpf/rpf9_scan/tests/stored.rs`. Left of them: a PlayStation 5 pack saves only as the file it is, and an edit of one is refused; the modules that read an archive by its file name still do; and `InstallLayout` stands.

## Measurements still owed

Each needs a PlayStation 5 table read, so each waits on a key snapshot for that build.

1. **Pairs.** Whether a row of `X.rpf` names a body in `X_concat.rpf`.
2. **Rows and chunks.** Whether a PlayStation 5 row is always stored, and which coders its chunks state.
3. **The mount.** Whether the registration documents name the archives as the Xbox's do.
4. **Readers.** Which headerless layouts differ, by a census of every reader over both builds.
5. **Programs.** The package of a PlayStation 5 program and its instruction set.
6. **Audio.** The codecs its containers state.

Two need no key: what the header byte at 0x2E selects, and the PlayStation 5 texel order, which R72d solves.

## Asked of the owner

Each has a default the rows take if nothing is said.

| Question | Default |
|---|---|
| Is a console build its own profile, with the title as an id above it? | yes |
| Is the Xbox build's id renamed so that the plain id is PC's? | not yet: what OpenRage keeps for a build is kept under its id |
| Does `Game::detect_from_root` stop answering GTA V Legacy for an unknown folder? | yes |
| Is `InstallLayout` deleted or made the mount's source? | deleted |
| Do the module move and the rename of the feature document wait for the other GTA VI lanes? | yes: R72j is last |
| Where does a PlayStation 5 key snapshot come from? | not from this plan |

## Risks

- **Other lanes edit the same files.** The archive folder is where the GTA VI rounds work. Rows a to g change behaviour in place and keep file names; only R72j moves files.
- **A rule measured on one platform may not hold on the other.** The header's shared rules passed on the PlayStation 5 headers read with the one byte forced; the nested ones were not all read. R72a's check reads every header of both builds.
- **The texel order may not be one order.** It may differ by format or by size, as the Xbox's does. The comparison with the Xbox image shows it texture by texture, and a texture it cannot order is refused by name.
- **Most of the PlayStation 5 build stays closed without keys.** Rows a to e give detection, packs and a scan; the archives' tables, and with them every named file, wait on R72h.
