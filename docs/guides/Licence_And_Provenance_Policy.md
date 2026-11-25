# Licence and provenance policy

What OpenRage's tree may take from the tools and sources around it, and what
the 2026-09 audit of the tree found.

This is the engineering half, roadmap
[R3](../direction/Roadmap.md#r3-licence-and-provenance-policy). The legal half, from the risk
of publishing to the questions for a lawyer, is
[legal and provenance](../direction/OR_IO_Specification.md#legal-and-provenance); what a
clone does not get is [Private_Tier.md](Private_Tier.md).

## The rules

OpenRage is MIT-licensed. Everything in the tree must be its own work,
third-party code under an MIT-compatible licence, or code ported with its
authors' permission, and anything not its own is named in
`LICENSE-third-party.md`.

1. **A fact about a file format is yours to state; someone's expression of that
   fact is not yours to take.**
2. **The retail bytes decide a layout.** Measure them, and use a reference to
   cross-check.
3. **Cite what a third party can check:** a public URL, a repository path at a
   pinned commit, a test, a corpus measurement. The leaked source is cited only
   in the gitignored docs/extra/ tier, and a comment in code never says that
   code came from it.

**One crate is the exception to "MIT-compatible".** `crates/io/or-media` may hold decoders ported
or vendored from open implementations under a stricter licence, FFmpeg's LGPL-2.1-or-later C and
NihAV's AGPL-3.0 Rust, wherever that is the fastest correct route to decoding a format with no setup
([owner, 2026-10-05](../direction/Project_Context.md#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05)).
It is a crate of its own so that `or-io` stays MIT. Each ported file opens with a header naming its
origin, upstream path, revision and licence; `LICENSE-third-party.md` lists it; and the crate's
`Cargo.toml` `license` field states the strictest licence it holds. The tool is not published soon,
and the licences are reviewed before any publication, with
[R66h](../direction/Roadmap.md#r66h-publish-the-crate-and-the-editor-together).

They rest on the owner's decisions in
[Project_Context.md](../direction/Project_Context.md#decisions) and the
[or-io decisions](../direction/OR_IO_Specification.md#owner-decisions): provenance is not a
gate (2026-09-20), except that a leaked-source citation fails the lint inside
the crates `or-io` publishes (R66b); this repository and the crate are published
together after the history is rewritten (D3, D8); CodeWalker and CodeX ports
stay on their authors' permission, with no written grant (D7); the lawyer
questions gate publication only (D14); and declarations take their names and
layouts from the GTA source, with no provenance attribute (D15).

## Fact versus expression

Copyright protects how a fact is written down, not the fact. The test for any
artefact: could it be produced from the file bytes alone, in your own
vocabulary, given enough files and time? A choice the bytes did not force is
expression: a field's offset is forced, a tool's name for it is not.

| Facts: state them and cite the measurement | Expression: take it only under a licence or permission, attributed |
|---|---|
| offsets, sizes, strides, alignment, padding extent, signedness, endianness | a tool's invented names (`Unknown_Ch`, `BlockLength`): mention one as a cross-reference, never adopt it as an identifier |
| magic and version numbers, enum values, flag bit positions | comments and prose, reworded ones included: rewording is not re-deriving |
| structural relations: this count sizes that array | code structure: class hierarchies, dispatch order, decomposition into functions |
| algorithms as mathematics: a hash, a compression scheme, a linear solve | constants the format does not force: a fallback colour, a default vtable value, a tolerance |
| behaviour observed by running the game or a tool | compilations: a table of thousands of names assembled by someone's labour |
| text the user's own files carry: name tables, PSO string tables, shader and material names | |

**Engine names.** RAGE's identifiers are Rockstar's vocabulary, not CodeWalker's
or CodeX's to grant. One name is weak subject matter; a large set taken from the
leaked source is arguably a compilation and evidence of access. `or-io`'s
declarations take their names from the GTA source anyway, by the owner's
decision D15, taken with that risk recorded in
[where the risk is](../direction/OR_IO_Specification.md#where-the-risk-is). The metadata
schema snapshot is not bundled (D5), and
`crates/io/or-io/src/gta5/script/natives.rs` reads a native-name list from the
user rather than containing one.

⚠ Reading an implementation you may not copy and writing "your own" version a
week later is a translation with a delay: a derivative work however it is
worded. It applies to the code and comments of Sollumz, RAGE-Tools, PyMateria,
OpenIV, NodeRunner Studio and the leaked source; take facts from them with a
measurement behind each. Measure and write the test before opening a reference.

## Sources

| Source | Licence | What the tree may take |
|---|---|---|
| The GTA V source (leaked) | Take-Two's, unlawfully published | Names, types, field order, padding and inheritance for declarations (D15), cross-checked against CodeWalker, the corpus and the testimony; and, since the owner's ruling of 2026-10-08, code ported for the renderer's frame, checked against the retail behaviour. A comment never says where code came from, and nothing cites the source by path, file or line outside docs/extra/ |
| CodeWalker | no repository licence, so never call it MIT-licensed; `Notice.txt` carries Neodymium's and Cameron Berry's MIT notices and Hamish Milne's GPL FbxWriter | The ported modules, with its authors' permission (D7), each named in `LICENSE-third-party.md`. Otherwise an oracle: run it and diff its XML against ours. Its FBX path is out, being GPL. `external/CodeWalker` stays a submodule, and nothing else of it is redistributed but its key snapshot, bundled in `or-io` by D18. Three issues asking it for a licence (#240, #243, #382) are unanswered; under D7 none is needed |
| CodeX | none stated | `or_io::meta`'s binary readers, ported with its authors' permission (D7) and listed in `LICENSE-third-party.md`. Cite a public URL, never a local path |
| szio | MIT, © 2025 Sollumz contributors | Code, with its notice kept and a `LICENSE-third-party.md` entry. Its `native` backend calls PyMateria; do not follow it there |
| Sollumz | GPL-3.0-or-later | Facts, each with a measurement. No code, names or comments |
| RAGE-Tools | a bespoke licence forbidding redistribution; a GPL-3 relicence landed on 2026-09-10 and was reverted on 2026-09-14 | Feature parity only: what users want a tool to do. It vendors `CodeWalker.Core`, so its format code is CodeWalker's under a further licence. Re-check the licence before relying on this row |
| PyMateria and Materia | proprietary and revocable: no bundling (§2.2), no competing products (§2.4), no reverse engineering (§2.6) | Install, run and benchmark against it and cite its published API (owner, 2026-09-20: OpenRage does not compete with it). Never bundled; none of its code enters |
| OpenIV | closed; its EULA forbids decompilation | Observed as a user. Its published `.oiv` package-format specification may be matched as a specification |
| RPFKit | unreleased, all rights reserved | Nothing to read |
| NodeRunner Studio | PolyForm Strict 1.0.0: reading and non-commercial use, no distribution and no work based on the software | Feature parity only: what a person can do with it. The inventory of [World_Editor_Parity.md](../direction/World_Editor_Parity.md) was read from it once, on 2026-11-25, and says each capability in OpenRage's own words. No code, text, table or asset enters the tree. That a lane which builds a capability does not open its sources is the plan's own caution and not an owner's ruling: the plan puts it to the owner as [ruling 18](../direction/World_Editor_Plan.md#rulings-asked-of-the-owner), with the default yes |
| FFmpeg (libavcodec, libavformat) | LGPL-2.1-or-later; what it builds only under `--enable-gpl` is GPL and is out | Code ported into `crates/io/or-media`, each file headed with its upstream path, revision and licence (owner, 2026-10-05). A Bink 1 `.bik`, picture and sound, and a Bink 2 movie's sound are decoded by that port ([R34c](../direction/Roadmap.md#r34c-video-preview)), and Legacy's XMA `.awc` streams by its port of the WMA Pro decoder ([R34a](../direction/Roadmap.md#r34a-the-audio-wave-container)); no FFmpeg program or library is run, found, bundled or linked by the app. An FFmpeg the owner has installed is the reference that three FFmpeg-gated tests, `every_shipped_bink1_matches_ffmpeg`, `hand_built_bink_audio_decodes_as_ffmpeg_decodes_it` and `legacys_eight_xma_streams_decode_as_ffmpeg_decodes_them`, hold the ports to, and the only place one runs |
| RAD's `bink2w64.dll` | proprietary, licensed to Rockstar and shipped with each GTA V install | Its disassembly and decompilation are read for interoperability, and what they show is written as Rust in `crates/io/or-media` alone: the in-loop filter of a Bink 2 picture (`crates/io/or-media/src/bink2/filter.rs`) and the clamp of a motion vector and the block records it needs (`crates/io/or-media/src/bink2/video.rs`), each file headed with the DLL's file, its version (2020.07, the Enhanced install's) and the addresses of the functions it follows, and listed in [LICENSE-third-party.md](../../LICENSE-third-party.md) (owner, 2026-10-06, [recorded](../direction/Project_Context.md#the-bink-2-in-loop-filter-is-derived-from-rads-dll-2026-10-06)). It is part of the review before publication, with [R66h](../direction/Roadmap.md#r66h-publish-the-crate-and-the-editor-together). No code or bytes of the DLL are copied into the tree and the app loads nothing. A test on Windows loads the DLL beside the executable of the user's own install at run time, as the reference `or-media`'s Bink 2 decoder is held to plane for plane ([R34c](../direction/Roadmap.md#r34c-video-preview)); it is never copied, bundled, redistributed or committed, like the keys taken from the user's own executable. Its exports and the fields an opened movie and its frame buffers hold are measured from the DLL itself, not taken from a header. The app decoded Bink 2 through it from 2026-10-03, until `or-media` decoded it itself |
| NihAV | AGPL-3.0 | Code ported into `crates/io/or-media`, each file headed with its upstream path, revision and licence (owner, 2026-10-05); its Bink 2 decoder, `nihav-rad`'s `bink2.rs`, was out under the 2026-10-03 entry, which that decision supersedes, and is ported there ([R34c](../direction/Roadmap.md#r34c-video-preview)) as AGPL-3.0-only, which its `COPYING` states with no later version in its README. What a ported AGPL file asks of the app and the crate that link it is reviewed before any publication, with R66h and the lawyer questions (D14) |
| nanomp3 and nanomp3-core | `MIT OR Apache-2.0`, © 2025 Robert B. Langer; a port of minimp3, which is CC0 | Linked as crates behind `or-io`'s `awc` feature to decode Enhanced's MP3 `.awc` streams ([R34a](../direction/Roadmap.md#r34a-the-audio-wave-container)), taken under MIT with its notice in `LICENSE-third-party.md`. Pure Rust, so `awc` still builds for wasm32. MP3's patents have expired, so decoding it needs no patent licence. A GPL or MPL decoder, such as Symphonia's, is out |
| dxbc-spirv | MIT, © 2025 Philip Rebohle | Vendored whole in `crates/tools/or-shader-front/vendor/dxbc-spirv`, unmodified, with its licence beside it and an entry in `LICENSE-third-party.md`. The DXBC to SPIR-V translator of the shader pipeline ([R58f](../direction/Roadmap.md#r58f-shader-baking-off-windows)), which DXVK 3.0 uses. C++, compiled by `or-shader-front` only under its `native` feature and run in a helper program the editor never loads (owner, 2026-10-07) |
| rusty_mp3 | Apache-2.0, © 2026 Mata Network | Linked as a crate behind `or-io`'s `awc` feature to encode an Enhanced MP3 `.awc` stream when it is replaced from a WAV ([R34a](../direction/Roadmap.md#r34a-the-audio-wave-container)), with its licence in `LICENSE-third-party.md`. Pure Rust with no dependencies, so `awc` still builds for wasm32. It is the one permissively licensed pure-Rust Layer III encoder with a stable release that a search of crates.io found on 2026-10-05: `oxideav-mp3` (MIT) holds a minimum-viable constant-bitrate encoder beside a decoder, and `glint-audio` (MIT) wraps C++ it carries. The LGPL encoders are out: LAME, shine and its Rust port `shine-rs` (LGPL-2.0), and `mp3lame-encoder`, which links LAME. rusty_mp3's own changelog dates its 1.0.0 2026-10-04, and it calls itself a ground-up rebuild with no copyleft; its source names LAME, FFmpeg and minimp3 as references to measure against and nothing here can show its authors took none of their code. MP3's patents have expired |
| The public GTA V script decompilers: `njames93/GTA-V-Script-Decompiler`, its fork `maybegreat48/GTA-V-Script-Decompiler`, and `Jaycadox/ysc-utils` | none: GitHub reports no licence for any of the three (checked 2026-10-03), so all rights are reserved | Inspiration for algorithms only, which copyright does not cover. Nothing was copied from any of them into `crates/io/or-io/src/gta5/script/decompile/`: no code, no names, no comments and no tables; the decompiler lifts the stack code and structures the control-flow graph by methods of its own ([R63](../direction/Roadmap.md#r63-the-script-disassembler-and-the-natives-database)) |
| The natives database: `alloc8or/gta5-nativedb-data` (`natives.json`, `natives_gen9.json`) and the crossmap `CSY0N/GTA-V-Crossmaps`'s `Enhanced Crossmap.h` | none: alloc8or's README says "strictly for educational purposes", and GitHub reports no licence for either repository (checked 2026-10-03) | Nothing enters the tree. The `.ysc` inspector and `openrage-cli script download-natives` fetch the files from their repositories into the user's configuration directory only when the user asks, recording each upstream commit, and never in a test; tests use small synthetic lists (owner, 2026-10-03, [R63](../direction/Roadmap.md#r63-the-script-disassembler-and-the-natives-database)) |
| bcpack-rs | `MIT OR Apache-2.0`, © 2026 Ahmet Dedeler; its entropy lanes port Microsoft's silicon-rans, MIT | Code, taken under MIT: the entropy layer and the BC1 mode of the coder GTA VI's texture packs store bodies through, in `crates/io/or-io/src/gta6/bcpack/`, each file with the header it asks a port to keep and both notices in `LICENSE-third-party.md`. The tables of the other modes are in no source and are fitted on the user's machine from the user's own packs, never committed |
| Mesa's ACO compiler and AMD's machine-readable instruction description | MIT, © 2018 Valve Corporation; MIT, © 2026 Advanced Micro Devices | The opcode numbers and mnemonics of `crates/tools/or-shader-front/src/rdna/opcodes.rs`, generated from ACO's table and corrected against AMD's description, which is not in the tree; the notice is in `LICENSE-third-party.md`. The decoder and the lowering are OpenRage's own |
| Leonida and other published notes on the GTA VI build | Leonida is GPL-3.0 | Prior art read for facts only, from its documentation and never its code: no code, names, comments or tables. The coder's lane widths of the modes other than BC1 come from it, and a pack's bodies decode by them or do not. ⚠ The save containers of `crates/io/or-io/src/gta6/save.rs` were laid out from published notes with no save to measure, which rule 2 does not let stand as a layout: they are stated as unverified in [GTA_VI_Xbox_Build.md](../features/GTA_VI_Xbox_Build.md#saves-and-mission-tables) until a save is measured |
| GTAMods wiki | public documentation | Facts, cited by page |
| Game files from the user's install | Take-Two's | Measure freely, commit never. Parsers read the user's install and bundle no fallback |

## What enforces it

- `scripts/lint_provenance.ps1`, from the pre-commit hook and hosted CI,
  fails on any tracked file with a RAGE extension or named
  `schema_snapshot.bin`, and warns about untracked files no ignore rule covers
  that look like game content by extension or by an RSC7, PSIN or RPF7
  signature. Its citation half matches leaked-source path shapes and
  source-file-and-line citations and fails on one in any tracked file,
  documents included, with `external/` and the vendored translator skipped, because the
  whole repository is published with the crate. Engine names are not a
  pattern (D15).
  What else keeps private content out is in
  [Private_Tier.md](Private_Tier.md#what-keeps-it-out-of-a-commit).
- `LICENSE-third-party.md` is the attribution record; nothing compares it with
  the tree. Nothing checks that a `.rs` file does not transcribe game content,
  which the extension check cannot see.

## Open items

- `or-media` holds the ports of FFmpeg n7.0's Bink 1 and Bink audio decoders and of its WMA Pro
  decoder, which decodes XMA, all LGPL-2.1-or-later, NihAV's Bink 2 decoder, AGPL-3.0-only, and the
  Bink 2 filter derived from RAD's `bink2w64.dll`; its `Cargo.toml` states the strictest of them,
  AGPL-3.0-only, and `LICENSE-third-party.md` lists each file. Each file ported later gets its row
  there, and every licence is reviewed before publication ([R66h](../direction/Roadmap.md#r66h-publish-the-crate-and-the-editor-together)).
- Nothing audits the Cargo dependencies' licences; `cargo-deny` is planned in
  [cleanup before publishing](../direction/OR_IO_Specification.md#cleanup-before-publishing).
- The module comment of `crates/io/or-io/src/meta/binary/public_names.rs`
  still calls CodeWalker MIT-licensed.

## The 2026-09 audit

The tree against this policy at `c4c73516` (845 tracked files), and what became
of each finding. R66b (#202, 2026-09-24) resolved the tree's findings, and
`LICENSE-third-party.md` listed the DXC redistributable until the old bake chain left the tree; what is still open is under
[open items](#open-items), and R66g removes what the history still holds
([rewriting the history](../direction/OR_IO_Specification.md#rewriting-the-history)).

| Finding at `c4c73516` | Now |
|---|---|
| 71 tracked files cited paths inside the leaked engine tree (46 its particle library's headers, 12 its serialisation declarations; 95 matched a broader engine-source shape), several to the line | removed; such citations fail the lint in any tracked file |
| Two doc comments reproduced engine code verbatim, a C++ serialisation function and a struct declaration | removed |
| A particle constant table stated its sizes came from the leaked headers | removed |
| The contributor guide ranked the leaked source first in its truth hierarchy | the retail bytes rank first, and the GTA source is the naming authority (D15) |
| `LICENSE-third-party.md` put Neodymium's 2015 notice over CodeWalker's later work, and several tracked places called CodeWalker MIT | the manifest names each module's upstream under D7; one code comment still says MIT ([open items](#open-items)) |
| `or-meta`'s binary readers called themselves CodeX ports, citing a local path | the comments are gone; the port is in the manifest's CodeX table |
| A 154,069-byte CodeWalker key snapshot, `magic.dat`, was embedded in every binary | removed, then bundled again in `or-io` behind `bundled-keys` by D18: it decodes only with the AES key from the user's executable ([keys](../direction/OR_IO_Specification.md#keys-d4)) |
| A 212-entry material table transcribed from the game's material files was compiled in as a fallback, with CodeWalker's fallback colour | removed; the palette is read from the user's install, and an unnamed material takes OpenRage's own grey |
| The DXC redistributable (the `dxc` folder of the tools directory): six `.lib` files with their headers and licence files, was tracked and absent from `LICENSE-third-party.md` | listed there with its licence files, then removed from the tree with the old bake chain |
| szio and Sollumz, the two cleanly licensed references, appeared nowhere in the tree; everything the code leaned on was unlicensed | unchanged: neither appears in code, and the ported sources are CodeWalker and CodeX, by permission (D7) |

## Harvesting names from an install

A name set for a title `or-io` does not declare yet
([R54d](../direction/Roadmap.md#r54d-a-pack-authored-from-outside-this-project)), a metadata
schema or a native list can be built from what the user's own install carries:
`.nametable` files inside archives, PSO and RBM string tables, `.sps` shader
preset and shader parameter names, material names, entity and archetype names
from `.ymap` and `.ytyp`, Jenkins reverse lookup against all of them, and szio
for what its authors published under MIT. It will not recover every
engine-internal name, and need not: coverage is a number, so harvest, count what
resolves, and judge the set by that.
