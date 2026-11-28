<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/openrage-full.png" width="180">
    <img src="assets/brand/openrage-symbol.png" alt="OpenRage" width="140">
  </picture>
</p>

# OpenRage

OpenRage opens, edits and saves the files Rockstar's RAGE games ship: a desktop editor, a command line and a Rust library, built from one workspace.

<p align="center">
  <img alt="status: pre-1.0" src="https://img.shields.io/badge/status-pre--1.0-orange">
  <img alt="platforms: Windows, Linux, macOS" src="https://img.shields.io/badge/platforms-Windows%20%C2%B7%20Linux%20%C2%B7%20macOS-blue">
  <img alt="rust: nightly" src="https://img.shields.io/badge/rust-nightly-lightgrey">
</p>

<p align="center">
  <a href="#games">Games</a> ·
  <a href="#formats">Formats</a> ·
  <a href="#the-apps">Apps</a> ·
  <a href="#build-from-source">Build</a> ·
  <a href="#documentation">Documentation</a>
</p>

<p align="center">
  <img src="assets/screenshots/model.png" alt="The Model workspace with a GTA VI folder open: its archives, a model in the viewport, the model's textures and its details" width="900">
</p>

## Who it is for

- **Modders.** Browse an install as one file tree, open a model, a texture, an animation or an effect, see it in a GPU viewport, change it and save it, into a `mods` copy when the archive is the game's own.
- **Content creators.** A project holds clothing, addon peds, weapons, maps and particle effects as editable sources, checks them against rules that say what is wrong and offer the fix, and builds a server resource, a DLC pack or an `.oiv` package ([guide](docs/guides/Workbench_Guide.md)).
- **Tool and pipeline authors.** `openrage-cli` does the same work with no window, its `serve` mode answers other programs over NDJSON, its `mcp` mode gives an AI agent the same as tools ([install](docs/reference/MCP_Server.md#install)), and a WebAssembly plugin adds panels and commands to the editor.
- **Rust developers.** `or-io` declares GTA V's structures under the engine's names, and a file opens as an ordinary value: read a field, assign it, call `save()`.

## Games

**R** reads, **W** writes, **–** not yet.

| | GTA V Legacy | GTA V Enhanced | Red Dead Redemption 2 | GTA VI |
|---|:-:|:-:|:-:|:-:|
| Install found and browsed | yes | yes | yes | yes, as a folder |
| Archives | RPF7 · R W | RPF7 · R W | RPF8 · R | RPF9 · R, W as a patched copy |
| Textures | R W | R W | – | R W, in part |
| Models | R W | R W | – | R, W in part |
| Collision and navigation | R W | R W | – | R, W in part |
| Animation | R W | R W | – | R, W in part |
| Particle effects | R W | R W | – | R, W in part |
| Maps and world data | R W | R W | – | R, W in part |
| Metadata and text | R W | R W | – | R, W in part |
| Audio | R W | R W | – | R, W in part |
| Video | R | R | – | R |
| Scripts | R | R | – | R, a value W |
| Viewport lit by the game's own shaders | yes | – | – | in part |
| The world streamed and flown through | – | – | – | yes |

- **GTA V.** Both editions are first class. A file's layout comes from its own RSC7 version, never from the install it came from, because an Enhanced install ships Legacy resources too.
- **Red Dead Redemption 2.** Its archives open; its resources are not decoded yet.
- **GTA VI.** An archive's table is enciphered, and its keys are read from the user's own executable. With them an install is browsed as a GTA V one is, a compiled script reads as decompiled source ([how](docs/features/GTA_VI_Script_Decompiler.md)), and the World workspace flies through the world as the game streams it ([how](docs/features/GTA_VI_World_Viewer.md)); without them an archive is listed by a scan of the bodies it stores in the clear. An edit saves as a file of its own or into a patched copy of its archive, which keeps the signed table byte for byte. The game refuses an archive whose table was rebuilt, and no file OpenRage saved has been tried in the game ([guide](docs/guides/GTA_VI_Modding.md), [what is read and refused](docs/features/GTA_VI_Xbox_Build.md)).
- **Earlier titles.** `or-io` also reads and writes the archives of the RAGE games before GTA V and the IMG files of GTA III to GTA IV (RPF0 to RPF6, IMG1 to IMG3), and nothing inside them.

[File_Support.md](docs/features/File_Support.md) says what each cell covers and where it stops.

## Formats

GTA V's files, in both editions.

| Kind | Files | Read | Write | In the editor |
|---|---|:-:|:-:|---|
| Models | `.ydr` `.ydd` `.yft` | yes | yes | Model workspace: viewport, materials, skeleton, glTF out and back |
| Textures | `.ytd` `.dds` | yes | yes | Texture workspace: mips, export, replace, BCn encoding |
| Particle effects | `.ypt` | yes | yes | Particle workspace: simulated preview and timeline |
| Animation | `.ycd` | yes | yes | Animation workspace: clips played on a skeleton |
| Animation data | `.yed` `.yfd` `.ypdb` `.mrf` | yes | yes | outline; a `.mrf`'s state machines and transitions |
| Collision and cloth | `.ybn` `.ybd` `.yld` | yes | yes | outline and inspector; `.ybn` and `.yld` in the viewport |
| Navigation and paths | `.ynv` `.ynd` `.yvr` `.ywr` | yes | yes | a navmesh's polygons in the viewport, disabled or deleted from the outline |
| Maps | `.ymap` `.ytyp` | yes | yes | metadata editor; a project's maps in the Map workspace |
| World tables | `heightmap*.dat` `waterheight.dat` `worldblanket.dat` `navprecalc.dat` `distantlights*.dat` | yes | yes | grids as images, lights as a list |
| Metadata | `.ymt` `.meta` (PSO, RBM, RBF0, XML), `.cut`, binary `.srl` | yes | yes | metadata editor |
| Text and tables | `.gxt2` `.nametable` `*_cache_y.dat` `metadata.dat` `audioworldsectorsinfo.dat` | yes | yes | text, or a table of records |
| Audio | `.awc`, `.rel` and its `.datNN` siblings | yes | yes | streams played, exported to WAV and replaced from WAV; a `.rel` is listed read-only |
| Interface movies | `.gfx` | yes | yes | tags and imports; SWF out and back |
| Archives | `.rpf` | yes | yes | browse, extract, replace, create, defragment |
| Video | `.bik` | yes | – | plays with its sound |
| Scripts | `.ysc` | yes | – | decompiled source and disassembly |
| Shaders | `.fxc` | yes | – | outline; what the Shaded viewport is baked from |
| Other | `.hdr` `ptxclipregions.dat` `materials.dat` | yes | – | |

Any other file opens as text when it is text, and read-only as its bytes otherwise. A save writes OpenRage's deterministic canonical form rather than the shipped bytes, and saving that again is byte-identical.

## The apps

### `openrage`, the editor

A desktop app on egui, winit and wgpu, with workspaces for browsing an install, models, textures, animation, particles and data, for a project's clothing, weapons and maps, and for flying through GTA VI's world.

- **Viewport.** Wireframe, Solid, Material and Shaded modes; Shaded draws with GTA V Legacy's own shaders, baked from the user's install, and a GTA VI model's passes with the game's own programs where they translate ([shading modes](docs/features/Viewport_Shading_Modes.md)).
- **World.** A free camera through GTA VI's world as the game streams it, at the hour and in the weather the World panel sets ([how](docs/features/GTA_VI_World_Viewer.md)).
- **Previews.** A particle simulator ([how](docs/features/Particle_Preview.md)), an animation timeline ([how](docs/features/Animation_Preview.md)), and audio and video decoded in OpenRage's own Rust, so nothing else needs installing.
- **Mods.** A shipped archive can be copied into the install's `mods` overlay, which the app then reads in place of the original, and a Mods panel lists what an install holds and in which order ([details](docs/features/File_Support.md#the-mods-overlay)).
- **Plugins.** A WebAssembly module asks for capabilities by name, the user answers before its first run, and every refusal is shown ([plugins](plugins/README.md)).
- **Languages.** English, Chinese, French, German, Italian, Japanese, Korean, Portuguese, Russian and Spanish ([guide](docs/guides/Interface_Languages.md)).

### `openrage-cli`, the command line

The same readers and writers for scripts; `openrage-cli <command> --help` gives each command's usage.

| To | Commands |
|---|---|
| Look inside | `inspect` `describe` `search` `depends` `names` `hash` `hashes` `formats` `versions` `analyse` |
| Convert | `convert` `meta` `texture` `mesh` `awc` `gfx` `script` `navmesh` `deflate` |
| Work on archives | `rpf` `bulk` `dlc` `dlc-list` |
| Work on GTA VI | `keys` `rpf9` `ttpf` `model` `gta6-project` `gta6-save` `gta6-mission` |
| Build content | `project` `wardrobe` `addonped` `weapons` `maps` `particles` `source` `world` |
| Automate | `serve` (NDJSON over stdio, [protocol](docs/reference/CLI_Server_Protocol.md)), `mcp` (for AI agents, [tools](docs/reference/MCP_Server.md)), `photo` (pictures of models with no window) |

```bash
openrage-cli rpf list x64a.rpf
openrage-cli texture export vehshare.ytd out/
openrage-cli mesh export police.ydr police.glb
openrage-cli meta to-xml peds.ymt peds.xml
```

### `or-io`, the library

```rust
use or_io::gta5::*;

let mut car = GtaDrawable::open("police.ydr")?;   // the layout comes from the file's RSC7 version
println!("{} {:?}", car.name, car.lod_group.lod_distances);
car.lod_group.lod_distances[0] = 45.0;            // plain assignment
car.lod_group.lods[0].models.remove(1);           // structural edits are Vec operations
car.save()?;                                      // validated, canonical form
```

Errors come only at open and at save. [OR_IO_Specification.md](docs/direction/OR_IO_Specification.md) is the design, and [crates/io/or-io](crates/io/or-io) the crate.

## Build from source

OpenRage is pre-1.0, and crate paths and APIs still move. [Releases](https://github.com/openragee/openrage/releases) hold a `.zip` for Windows, a `.tar.gz` and a `.deb` for Linux and an unsigned `.dmg` for macOS, each with a checksum. Windows is the primary platform; on Linux, sound needs ALSA's headers to build (`libasound2-dev`). The toolchain is the nightly [rust-toolchain.toml](rust-toolchain.toml) pins. Install [just](https://github.com/casey/just) and `cargo install cargo-nextest --locked`, then:

```bash
just build        # release build of the whole workspace
just dev          # run the editor
just self-check   # draw every panel through real frames, with no window
just ready        # before a merge: clippy and the default tests, over what the change touches
just test         # every test of the workspace: about twenty minutes
```

No game file is committed. OpenRage works on an install the user owns and takes the keys it can from that install's executable, and the suites that read real files skip on a clean checkout, which is not a pass ([staging a corpus](crates/io/or-io-verify/tests/fixtures/README.md)).

The repository is mirrored on [Radicle](https://radicle.xyz) as `rad:zhdibp82Y5a4zqi44zZuJA6mt8tm`, which follows GitHub within about ten minutes: `rad clone rad:zhdibp82Y5a4zqi44zZuJA6mt8tm`.

## Repository layout

Dependencies run one way, foundation → io → title → domain → runtime → app, and every crate has a README.

| Layer | Holds | Crates |
|---|---|---|
| app | the editor and the `openrage-cli` entry point | [src](src) |
| [runtime](crates/runtime) | what is evaluated, simulated or drawn | or-anim, or-renderer, or-simulation |
| [domain](crates/domain) | the models of what is edited, and the kinds of content a project builds | or-project, or-source, or-content, or-wardrobe, or-addonped, or-weapons, or-maps, or-particles, or-particle, or-world, or-world-gta5, or-world-gta6, or-worlds, or-depends, or-interchange, or-editor, or-plugin |
| [title](crates/title) | a game install: editions, archives, DLC order | or-title |
| [io](crates/io) | file formats, archives and media decoders | or-io, or-io-macros, or-io-verify, or-kraken, or-media |
| [foundation](crates/foundation) | building blocks that know no format | or-job, or-transport, or-test-support |
| [tools](crates/tools) | programs beside the chain | or-cli, or-shader-tool, or-shader-front, xtask |

[Crate_Reference.md](docs/reference/Crate_Reference.md) describes each crate and its traps.

## Documentation

- [docs/](docs/README.md) indexes every document, and [the roadmap](docs/direction/Roadmap.md) says what is done and what comes next.
- [AGENTS.md](AGENTS.md) holds the rules every contributor and agent session follows, and [the development workflow](docs/guides/Development_Workflow.md) how a change lands.
- [The format guide](docs/guides/New_Format_Implementation_Guide.md) brings a new format up from reconnaissance to the app.
- [The GTA VI modding guide](docs/guides/GTA_VI_Modding.md) goes from the keys of an executable to a patched copy of an archive, and [the acceptance sheet](docs/guides/In_Game_Acceptance.md) says what to look for in a running game.
- RAGE's binary layout specifications are held out of the repository ([Private_Tier.md](docs/guides/Private_Tier.md)).

## Licence

MIT — see [LICENSE](LICENSE). Third-party code and its notices are listed in
[LICENSE-third-party.md](LICENSE-third-party.md). The one exception is `crates/io/or-media`,
which decodes the audio and video: it holds code ported from FFmpeg (LGPL-2.1-or-later) and from
NihAV (AGPL-3.0), and a Bink 2 filter derived from RAD's proprietary `bink2w64.dll` for
interoperability, so it is AGPL-3.0-only. The editor and the CLI link it, in a release's packages
too, and what that asks of them is not reviewed yet ([policy](docs/guides/Licence_And_Provenance_Policy.md)).
The decoder of BCPack texture bodies is ported from
[bcpack-rs](https://github.com/Ahmet-Dedeler/bcpack-rs) by Ahmet Dedeler.

OpenRage is an unofficial, independent project. It is not affiliated with,
endorsed by, or connected to Rockstar Games or Take-Two Interactive. "Grand
Theft Auto" and "Red Dead Redemption" are trademarks of their respective owners.
No game assets are distributed with this software; it operates on files the user
already owns. OpenRage extracts the keys it can from the user's own game
executable. The rest come from the key snapshots OpenRage bundles, CodeWalker's
for GTA V among them, or from one the user supplies; a bundled snapshot is
encrypted under a key taken from the user's executable, so it opens nothing
without the game.
