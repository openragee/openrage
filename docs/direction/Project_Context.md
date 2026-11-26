# Project context

What OpenRage is for, the vocabulary its design rests on, and the owner's dated decisions, newest first: read it before reopening a settled question. What is done and what is next is in [the roadmap](Roadmap.md).

## What OpenRage is

A replacement for CodeX, CodeWalker and OpenIV: an app that understands the files of Rockstar's titles. Those are GTA V in its Legacy and Enhanced editions, Red Dead Redemption 2 (its RPF8 archives open; its RSC8 resources do not decode yet) and, in future, GTA VI.

The core generalises create, edit and delete across Rockstar formats rather than going deep into one domain, and exposes a plugin surface. The model is a mix of an extended Windows Explorer for Rockstar titles and Blender. In the app a plugin is WebAssembly; outside it, a plugin is a subprocess speaking the CLI's NDJSON protocol ([2026-09-21](#a-plugin-is-wasm-in-the-app-and-a-subprocess-outside-it-2026-09-21)). A clothing tool equivalent to DurtyClothTool, built on OpenRage rather than CodeWalker, is a later goal and the reference case for the in-app surface.

## Vocabulary: container, title and target

Three independent properties, kept apart so that a declaration or a data file can supply a layout:

- **Container**: RSC7 or RPF7, a property of the file.
- **Title**: the game and edition an install holds (GTA V Legacy, GTA V Enhanced, RDR2), a property of the install; `or-title` models each as a `GameProfile`.
- **Target**: Gen8 or Gen9, a property of the individual resource, read from its own RSC7 version and never from the install. An Enhanced install ships Gen8 resources.

## Architecture principle

A RAGE byte layout exists in one place. Since 2026-09-24 that place is the `or-io` crate, to be published: one declaration per structure, named and laid out from the GTA source, from which the owned value, the raw layout with its opt-in view and the built-in description are generated ([OR_IO_Specification.md](OR_IO_Specification.md)). No other layout is read: [R66](Roadmap.md) moved the consumers onto the owned values (R66e) and deleted `or-asset`, `#[rage_struct]` and pack discovery (R66f), and every target of every declared format opens, saves and authors a blank with no pack (`every_declared_format_opens_with_no_pack`).

⚠ A wrong layout is worse than a missing one: plausible numbers read from wrong offsets make files the game crashes on, with nothing visible to warn you.

## Decisions

Newest first. An entry records a decision as it was taken; one a later decision overturns says so on a `Superseded` line. or-io's D1–D18 are in full, with the owner's words, in [the specification's table](OR_IO_Specification.md#owner-decisions).

### The world editor plan is built in parallel lanes (2026-11-25)

After [the plan](World_Editor_Plan.md) was written, the owner set the goal: "start on it on a parallized workflow, when a workflow is near the end start another with an actionable task list".

- **What it starts.** The rows of [R70](Roadmap.md#r70-the-gta-v-world-editor), each by the default beside its question in [the rulings asked](World_Editor_Plan.md#rulings-asked-of-the-owner), each in a lane of its own. When a set of lanes nears its end, the next starts from a task list.
- **What it does not decide.** A default that reopens a dated decision or builds what [Workbench_Plan.md](Workbench_Plan.md#held-back) holds back waits for the owner's answer: the interior builder, the walk mode and the thumbnail browser in the app and not a plugin; reading the XML a Blender export writes; a drawable made from nothing; a typed root declared by hand outside the families the record names; and installing into the game's mods folder.
- **Nothing is pushed or merged.** The GitHub credentials on the machine were invalid that day, so each lane is a local branch.
- **Lanes at once.** More than three lanes compiled at once, on a machine of 30 logical CPUs. That is above the three of [2026-09-26](#parallel-lanes-three-compiling-five-built-2026-09-26), a figure sized for a machine of eight, which the owner may restate.

### Every NodeRunner Studio capability for GTA V is wanted in OpenRage (2026-11-25)

The owner pointed at NodeRunner Studio (`https://github.com/HypnoZDev/NodeRunner`), a GTA V map editor for FiveM, and wrote: "Je veux toutes les fonctionnalités sur openrage pour gta v", asking for a plan of every feature, what makes each product good, the case for and against, and "comment bien intégrer les choses dans openrage, intégrer le world editor, le système de projets, avec une bonne UX pour les mappers", as a detailed roadmap.

[World_Editor_Plan.md](World_Editor_Plan.md) is that plan and [R70](Roadmap.md#r70-the-gta-v-world-editor) schedules it; the earlier games NodeRunner shows stay with [2026-10-01](#earlier-rage-titles-are-a-future-goal-2026-10-01).

The request rules on nothing else. The plan asks for rulings ([the rulings asked](World_Editor_Plan.md#rulings-asked-of-the-owner)) and this entry gives none; these touch what is recorded:

- The interior builder, the walk mode and the thumbnail browser in the app: [2026-10-03](#the-ux-reviews-plan-is-the-apps-next-work-2026-10-03) left the world editor a plugin, and the ruling of 2026-10-20 in [Workbench_Plan.md](Workbench_Plan.md#rulings-in-force) moved placement into the app.
- Reading the XML a Blender export writes, against the XML ruling of 2026-09-22 ([R37](Roadmap.md#r37-metadata-and-xml-at-parity)), and a drawable made from nothing; [Workbench_Plan.md](Workbench_Plan.md#held-back) holds both back.
- Key presets, which 2026-10-03 left to the owner.
- Installing a story pack into the game, undecided since [2026-11-21](#the-window-becomes-tabs-and-a-rail-in-ten-rows-2026-11-21).
- The interior records in `or-io` and a story pack's nested archive, asked on 2026-10-21 and not answered; a typed root by hand outside the families [2026-09-26](#world-metadata-roots-may-be-declared-by-hand-2026-09-26) names.

The owner gave no ruling on NodeRunner's licence, PolyForm Strict 1.0.0, which forbids a work based on the software ([Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources)).

### Hosted CI is on again, and the self-hosted workflows stay off (2026-11-23)

The owner asked, of the public repository: "Make sure the openrage CI pass, and that on a tag it publish a github release with all the builds". `ci.yml` is enabled again (`gh workflow enable`), and `release.yml` published `v0.1.0` with a package for each OS. `local-gate.yml` and `main-tip.yml` stay disabled, because the repository has no self-hosted runner: no automatic check sees the corpus, and `just ready` with a format's `just test-format` stays the gate before a merge.

The owner then had `ci.yml` start on every push to `main` as well as on a pull request, as the decision of 2026-10-06 intended: `main-tip.yml`, which checked each landed commit, has no runner, and a hosted run costs nothing on a public repository. The Windows and macOS jobs keep their weekly run.

Supersedes [2026-10-06](#ci-is-switched-off-during-the-dev-phase-and-runs-on-every-push-to-main-after-it-2026-10-06) for `ci.yml`; the other two workflows stay as that entry left them.

### The window becomes tabs and a rail, in ten rows (2026-11-21)

The owner asked for a review of every view and of the navigation with nothing held as fixed, and of the two restructures it proposed chose the bold one, built in stages ("A, staged"). [R69](Roadmap.md#r69-one-window-of-tabs-and-a-rail) is the plan.

- What is worked on is a tab: a file, a folder, the project's overview and each content of a project. The panels arrange themselves by the kind of the tab on screen, so the workspace strip, its More and the pinned buttons go.
- A rail of Home, Game, Project and Automate says where things come from, one Details panel shows what is selected, one Undo acts on the tab on screen, and Build is one control of the window.

It overturns rules 1 and 3 of [the UX design's verdict](Workbench_UX_Design.md#verdict) (nothing changes without a project; the workspace buttons are the only view switch), that a template chooses the buttons shown, and one clause of [2026-10-07](#projects-sources-and-the-clothing-tool-are-built-as-one-plan-with-content-and-builds-on-screen-2026-10-07): Ctrl+1 to Ctrl+9 go to a tab. Each stands in the tree until its row lands. It takes two options the review of [2026-10-03](#the-ux-reviews-plan-is-the-apps-next-work-2026-10-03) left open, a sidebar switcher and copy as CLI command.

It reopens none of these: a workspace is a layout preset and a camera is per document ([2026-09-23](#each-document-keeps-its-own-camera-a-workspace-stays-a-layout-preset-2026-09-23)); Content and Builds; Open, Import and Add; a project that has no type and holds sources; a launch that reopens the last project.

Not decided: whether a button labelled Save to mods may write the overlay in Read only ([R69c](Roadmap.md#r69c-one-build-control-and-a-save-that-offers-what-can-happen) builds what needs no ruling), and installing an `.oiv` into the game, which no row schedules.

### An agent tests what its change touches, and a default run takes thirty seconds (2026-11-20)

Told that each new lane compiled cold, that `just ready-full` took seven to ten minutes a run and that the last gate had run twice, the owner said: "optimize the workflow for multi agentic work this is not sustainable, optimize the workflow so that when an agent want to test he only test the minimum he need to, and optimize just ready full so that it takes max 30s. default tests should never take more than 30S".

- **The gate is scoped to the change.** `just ready` checks the packages whose files differ from `origin/main` and the packages that depend on them; `just ready-full` is the same over every package.
- **The tests are two tiers.** The default tier is what every gate and a bare `cargo nextest run` take; the slow tier, the corpus walks, the install suites and the plugin guests, is named in `.config/nextest.toml` and only `just test` and `--profile all` run it. A change to a format's read or write path adds that format's `just test-format MODULE` to its gate.
- **`just test` is run on `main` between waves and is no lane's gate.** This is what the 30 s costs: a change can land that fails a slow-tier test, and it is found by the next run on `main`, not before the merge.
- **The separate self-check step is gone.** The app's tests run the same self-check in a process of its own, and that test stays in the default tier.

What 30 s does and does not cover is measured in [Development_Workflow.md](../guides/Development_Workflow.md#the-gate-and-the-two-tiers): a built lane with nothing to recompile is inside it, and an edit pays for the compiler on top, which for `or-io` is minutes.

### The game source may be ported, and a comment never says so (2026-10-08)

Asked how the viewport could mimic GTA's render exactly, the owner said to use as much of the install as possible and the game's source for the exact rendering code, and then, on the plan that follows (the timecycle's inputs, the deferred G-buffer and lighting, shadows, SSAO and the forward buckets, the post chain, then the extras, each a pull request shown with `just shots`): "you can copy code from source it's fine, it's just the comments in code should not state that provenance". Code ported from the source is therefore allowed in the renderer's frame, checked against the retail behaviour and the pack's own reflection; a comment says what the code does and never where it came from; nothing cites the source by path, file or line outside the gitignored docs/extra/ tier, which the provenance lint keeps failing on. **Superseded** in part: the rule "never its code or comments" of [the licence and provenance policy](../guides/Licence_And_Provenance_Policy.md), for code only; the comment rule and the citation rule stand, and [D15](OR_IO_Specification.md#owner-decisions)'s names for declarations are unchanged. The licence risk of ported code is the owner's to carry, as D15's was.

### The viewport's Shaded button draws with the game's shaders, and the renderer's own deferred pipeline is gone (2026-10-08)

The owner, looking at the viewport's four modes (Solid, Material, Shaded and Game shaders), asked for the game's shaders, accurate, to be the Shaded button and for the old shaded pipeline to be removed. Shaded now draws the open drawable and the particles through the SPIR-V pack, and the deferred path that drew the old Shaded mode (G-buffer, lighting, sky, bloom and tonemap, particle shadows, volumetric fog, decal splats, particle lights, and the world background's sky, gradient and HDRI) is deleted from `or-renderer` and the app. The background is one colour. [Viewport_Shading_Modes.md](../features/Viewport_Shading_Modes.md) says what the mode draws. What the old path drew with the renderer's own WGSL and the game's passes do not yet draw (lights, fog volumes, decals, shadows, the time of day's light) is listed in [Particle_Shader_ABI.md](../reference/Particle_Shader_ABI.md#known-gaps); it returns, if at all, through the game's own passes. This overturns nothing; [R58f](Roadmap.md#r58f-shader-baking-off-windows) is the plan this finishes.

### A shop's own metadata is imported, and a garment's cloth is linked and never declared (2026-10-08)

The owner ruled, while reading wave H's open questions, that the shop hashes of a pack must not be Durty Cloth Tool's alone: servers that migrated have their own shop metadata and their own entry hashes, so a project can import one file, several, or a folder searched recursively, and an imported shop metadata file is parsed, its entries are matched to the garments they name, and the hashes it states become the defaults. He also ruled that a garment does not declare that it has cloth: its cloth is linked in the project, as a child when the game allows one garment per cloth or as a field when it allows several, and the flag that says a garment has cloth follows from whether one is linked. He wanted a file watcher for the Wardrobe in place of its three-second rescan, and named a pack of his server as the reference to hold the engine to. [Workbench_Plan.md](Workbench_Plan.md) wave H3 builds this. It overturns nothing; the Durty-compatible option stays, off by default.

### Every RPF and IMG generation reads and writes in or-io (2026-10-07)

The owner's instruction, in his words: "Add RPF0 to RPF8 and IMG1 to IMG3 using available info you can find online about how they work, RPF7 is already included the DX should be the same as in RPF7 it's just other layers you need to add", with reads and writes, and "find a way to add writes for Red dead redemption 2 too", the install named. `RpfVersion` and the layouts under `crates/io/or-io/src/container/rpf/layout/` are that: one reader, one builder and one `Defrag` for every generation ([Crate_Reference.md](../reference/Crate_Reference.md#the-archive-and-the-version-registry)). There is no RPF1 or RPF5, and no public record of either.

The layouts come from public notes ([Swage's RPF notes](https://github.com/0x1F9F1/Swage/blob/master/src/games/rage/rpf.md), the GTAMods wiki's [RPF](https://gtamods.com/wiki/RPF_archive) and [IMG](https://gtamods.com/wiki/IMG_archive) pages, and [rpf-archive-rs](https://github.com/VIRUXE/rpf-archive-rs), which is public domain), stated as facts and written as code of this tree; no code was copied. Only RPF7 and RPF8 have a retail file here. The others are checked against archives assembled by hand from the published layouts, so a layout that a note gets wrong would pass them: say which generation before trusting one on a retail archive.

How RDR2 is written: an RPF8 table with header tag `0xFF` is in the clear, as an install's `shaders_x64.rpf` is, and a body whose key id is `0xFF` is too. A new archive is written that way, and an existing one is rebuilt around its bodies as stored (`insert_stored`), so a body in Oodle, which nothing here decodes, and one a key encrypts are carried and never touched. What it does not do: read the other twenty-three of an install's twenty-four top-level archives, whose tables the title's TFIT keys encipher; the keys are the user's own executable's to give, and no extractor for them exists. It writes the 256-byte RSA block as zeros, and whether the game accepts an archive whose block is not Rockstar's is unmeasured. Community tools also rewrite a copy of a nested archive's table that the game keeps in an index file of its own (`pfm.dat`, inside an archive); a rebuild here does not, and what the game does without it is unmeasured too.

**Added 2026-10-08:** the other twenty-three archives read. Their cipher is a table-driven block cipher chained like CBC, whose tables a snapshot holds, masked and encrypted under the AES key that every Rockstar executable carries, so `RDR2.exe` unlocks it as `gta5.exe` unlocks the GTA V snapshot. The owner supplied CodeX, a community tool that holds such a snapshot and its reader, and ruled the same day, in his words "yeah bundle the rdr2_magic.dat", that `or-io` ships it as D18 ships CodeWalker's: `crates/io/or-io/data/rdr2_magic.dat` behind `bundled-keys`, inert without the AES key from the user's own `RDR2.exe`, with the user's own snapshot (`OPENRAGE_RDR2_SNAPSHOT`, or `rdr2_magic.dat` in the configuration folder) tried first (`Rdr2CryptoProvider`). `the_key_snapshot_is_in_the_tree_once` holds the tree to that one copy of each. It carries the same lawyer question 4 as D18. He also asked for Oodle bodies to decode, and to write, on every platform: Kraken is read through the pure-Rust `oozextract`, and LZB16, the codec most of the title's small files use, is OpenRage's own, read and written, checked against the output of the game's own `oo2core` ([Roadmap R56](Roadmap.md#r56-rdr2-the-second-container)).

**Superseded** in part: [R56](Roadmap.md#r56-rdr2-the-second-container)'s reasoning that an archive-format trait was declined because RPF8 stores name hashes, and the non-goal "Writing RPF8", which this removes. [R57b](Roadmap.md#r57b-earlier-rage-titles)'s containers are done; what they hold is not.

### An archive is opened, edited by path and saved, whatever its generation (2026-10-07)

The owner, asked whether the DX of the archive code was the best it could be: "ok yeah improve the DX do not hesitate to make breaking changes and remove old code". The surface the generations were first added behind was RPF7's, which opened in three steps (a reader, its keys, `into_archive`), addressed a file by entry index with the keys passed again to every read, and could change one file only by re-adding every other to a fresh builder (the CLI and the app each carried that code). It is replaced, with no shim, by one that reads the same for every generation: `RpfArchive::open` and `from_bytes` (and their `_with` forms for keys) hold the keys they were opened with; `read` and `find` address a file by path, `read_entry` and `read_form` by index; `edit` returns an `RpfBuilder` holding every file, which `insert`, `remove` and `rename` change by path and `save` writes over the file it came from only once all of it is written. `RpfReader`, `load_rpf`, `save_rpf` and the `extract_*` family are removed.

The builder keeps paths, not a table. That fixes a defect RPF7's builder had since it was written: adding a path that already held a file silently kept the old file. A path is now matched without regard to case or slash, adding to it replaces, a file under a file is refused, and the table is laid out from the sorted paths at save, so the same files give the same archive in whatever order they were added. The one visible change to RPF7's bytes is the order of its name heap, which now follows the sorted paths and no longer the order files were added in.

### Projects, sources and the clothing tool are built as one plan, with Content and Builds on screen (2026-10-07)

The owner asked for projects to be implemented properly in OpenRage, with the advantages of Durty Cloth Tool, grzyClothTool and Vestium and none of their costs, serving the general modder and the clothing creator who comes from Durty Cloth Tool, and then said to launch the workflow on the final roadmap with Sonnet 5.5 agents at the highest effort. [Workbench_Plan.md](Workbench_Plan.md) is that roadmap and [Workbench_UX_Design.md](Workbench_UX_Design.md) draws the screens.

In the conversation that produced them he ruled: a project has no type and holds modules and outputs, shown on screen as Content and Builds, with templates and later links; a model is stored as glTF with a JSON file and imported from glTF, GLB or FBX; a texture is stored as an exact DDS and compiled from DDS or images with sidecar rules, channel packing and no convert button; a folder takes a word for its kind and no platform letter; a saved source is recooked live ("Live reload add it of course it should be supported"); Ctrl+B builds and Ctrl+1 to Ctrl+9 switch workspace; a launch reopens the last project with the start page one action away; one project lives in one window and a second project opens in a second window for now; Open folder offers to make the folder a project.

The plan takes the studies' recommendation for each question he did not answer one by one, and lists them as defaults a sentence can reverse. It holds back what needs a ruling: CodeWalker XML import, author mode, the shipped shader and vertex presets, and any claim that a build loads in game. This overturns nothing; R44f's text and R46's text stand until the wave that narrows R46 lands.

Superseded on 2026-11-21 by [the window becomes tabs and a rail](#the-window-becomes-tabs-and-a-rail-in-ten-rows-2026-11-21) for one clause: Ctrl+1 to Ctrl+9 go to a tab once [R69e](Roadmap.md#r69e-a-tab-for-each-thing-and-no-workspace-strip) lands.

### Game shaders are translated once to SPIR-V and run natively on each OS (2026-10-07)

The owner approved the plan in [Shader_Pipeline_Design.md](Shader_Pipeline_Design.md), choosing at each question: the scope is the whole frame, phased with the material preview first; the C++ translators live in a separate helper program that the editor never loads and the frame loop never calls, on his condition that "we need to not make ping pong between rust and c++ otherwise it will harm perf"; and RDR2 is designed for and gated on [R56](Roadmap.md#r56-rdr2-the-second-container), which has to open its archives first.

The pipeline translates each game's bytecode to SPIR-V with dxbc-spirv and dxil-spirv, then runs it through Vulkan passthrough on Linux and Windows and through naga on Metal, with DirectX 12 as the Windows fallback. He asked whether Vulkan or DirectX 12 was the better route, whether Slang should be the pivot and whether the bake is lossy; the design's tables of rejected alternatives and of backends, and its fidelity section, answer them.

This overturns nothing. [OD-OTHER-TITLES](#no-runtime-pack-for-another-title-2026-09-26) concerns the retired `.orpack`, not the shader pack, and the refusal off Windows that [R58f](Roadmap.md#r58f-shader-baking-off-windows) records stays until the helper replaces the Windows tools.

### The Bink 2 in-loop filter is derived from RAD's DLL (2026-10-06)

The owner's decision with W15-BINK2, on what [R34c](Roadmap.md#r34c-video-preview) had left open: the
in-loop deblocking filter of a Bink 2 picture, which no open decoder implements, and the clamp that
keeps a motion vector on the picture. OpenRage derives both from the disassembly and decompilation of
RAD's proprietary `bink2w64.dll`, for interoperability, so that its pictures equal the game's. The
owner opened the Enhanced install's copy, version 2020.07, in a Ghidra project for the work, and the
Legacy install's, 1.994a, is held to the same pictures.

- **Where it lives.** Only in `crates/io/or-media`: `crates/io/or-media/src/bink2/filter.rs`, and the clamp and the block
  records in `crates/io/or-media/src/bink2/video.rs`. Never in `or-io`. Each file opens with a header naming the DLL file,
  its version and the addresses of the functions it follows.
- **What it takes.** What the code does and the constants and tables it holds, written as Rust. No
  code or bytes of the DLL are copied, committed, bundled or loaded by the app: the Windows test that
  loads the DLL from the user's own install stays the reference the decoder is held to.
- **Where it is listed.** [LICENSE-third-party.md](../../LICENSE-third-party.md) names each file with the
  DLL, its version and the addresses, and the
  [policy's row for the DLL](../guides/Licence_And_Provenance_Policy.md#sources) says what enters the tree.
- **Reviewed before publication.** It is one of
  [R66h](Roadmap.md#r66h-publish-the-crate-and-the-editor-together)'s licence checks, with the AGPL
  and LGPL ports, and the lawyer questions of D14.

Supersedes the line in R34c that reading the DLL's own code for the filter was a licensing question the
owner had not ruled on.

### CI is switched off during the dev phase, and runs on every push to `main` after it (2026-10-06)

The owner's decision, in his words: "the ci should always run on branch main on push, but we are in
dev phase so for now the best is to completely disable the ci its unneeded for now". The three CI
workflows, `ci.yml`, `local-gate.yml` and `main-tip.yml`, are disabled in the repository's settings
(`gh workflow disable`, which leaves each file as it is); `release.yml`, which runs on a `v*` tag
only, is not. The intent is unchanged: when the dev phase ends, `gh workflow enable` restores each,
and CI runs on every push to `main`.

- **Until then `just ready` is the whole gate.** The merge rule's "CI and `layouts` are read
  afterwards, on `main`" has nothing to read, and no agent needs to cancel a run, because none starts.
- **The end of the dev phase is the owner's to call.** The hosted-CI cost decision of
  [2026-09-23](#hosted-ci-is-linux-per-pull-request-the-windows-suite-runs-on-each-tip-of-main-2026-09-23)
  is untouched and moot while the workflows are off; the cadence of the macOS job
  ([Appendix B](Roadmap.md#appendix-b--judgement-calls-to-confirm), item 6) is a question for the day
  they come back.

Superseded in part on 2026-11-23 by
[hosted CI is on again](#hosted-ci-is-on-again-and-the-self-hosted-workflows-stay-off-2026-11-23):
`ci.yml` is enabled, and the other two stay off for want of a runner.

### Linux and macOS are supported in full, and the tooling is Rust that runs the same on every OS (2026-10-06)

The owner's decision, in his words: "Write a plan to fully add to linux / macos support", and on
how the dev tooling should work: "Do the best cross platform way to do it, dont limit yourself by
current code feel free to find ways the most elegant , resepecting DRY, without considerins too
much breaking stuff". He chose every tier offered: the build, CI and correctness; feature parity
with Windows; shader baking off Windows; and packaging with a release. macOS ships unsigned
(ad-hoc signed) for now. [R58](Roadmap.md#r58-cross-platform) is the programme, in parts R58a to
R58g.

- **One implementation of each task, in Rust.** A new `xtask` crate holds what the PowerShell
  scripts do: the chunked formatter, the encoding and provenance lints, the hook, the lane and
  janitor commands and the recipes' inline logic. The `justfile` keeps one-line recipes that parse
  the same under `sh` and PowerShell, so the shell stops mattering. The agent limits stay
  PowerShell: they are Windows Job Objects and have no equivalent elsewhere.
- **The pre-commit hook runs a built binary.** AGENTS.md says the hook compiles nothing. It now
  runs the `xtask` binary that `just install-hooks` builds into `target/`, and refuses, saying to
  run that recipe, when the binary is missing or older than its source, in place of today's silent
  skip of the encoding and provenance lints when no PowerShell is on `PATH`. The rule's purpose,
  that a commit never waits on the workspace compiling, holds. AGENTS.md changes with R58a.
- **The old scripts go after one green run.** They stay until a `main-tip.yml` run on the
  self-hosted runner has used the Rust, because that runner gates every merge; a small follow-up
  deletes them.
- **How often the macOS job runs is not decided.** It runs weekly and on request, which is the
  agent's default by analogy with the Windows job under the cost decision of
  [2026-09-23](#hosted-ci-is-linux-per-pull-request-the-windows-suite-runs-on-each-tip-of-main-2026-09-23).
  That decision is about Windows and says nothing of macOS, and the owner has not ruled on it
  ([Appendix B](Roadmap.md#appendix-b--judgement-calls-to-confirm), item 6).
- **Never distributed.** A shader pack baked from the user's own install may be exported and
  imported between the user's machines; it holds game-derived data and is never committed or
  published.

### Every media format is decoded in Rust inside the tree (2026-10-05)

The owner's decision with the W15 wave, in his words: "We should write all code of it if you find
the code everything is open you can write we will not publish the tool any soonn anyways so we
probably will check back later but for me i need a proper tool, so write in rust all and if
needed use libs/crates that we can vendor". OpenRage decodes every audio and video format it
meets in its own Rust code, with zero setup on every platform, porting or vendoring open
implementations, FFmpeg's LGPL C and NihAV's AGPL Rust, wherever that is the fastest correct
route. The tool is not published soon, so the licence of each ported file is recorded where it
lives and reviewed before any publication, as one of
[R66h](Roadmap.md#r66h-publish-the-crate-and-the-editor-together)'s checks.

- **Where it lives.** `crates/io/or-media`, layer io, which may depend on `or-io`. Ported code
  stays out of `or-io`, so that crate's MIT licence stays clean. The `license` field of
  `or-media`'s `Cargo.toml` states the strictest licence the crate holds, which is
  AGPL-3.0-only since the Bink 2 port landed, over the LGPL-2.1-or-later of the Bink 1 port; each ported file opens with a header naming its origin, its upstream path, the
  revision it was taken at and its licence; and
  [LICENSE-third-party.md](../../LICENSE-third-party.md) lists each one.
- **One interface.** A source opened from bytes states its codec, channels, sample rate, frame size
  and rate, and duration, and opens its sound as interleaved samples or its picture as RGBA frames,
  seekable where the format allows. One table maps a codec to its decoder: a codec added later is
  one row, and a codec with no row is refused by name. The `.awc` codecs `or-io` decodes, PCM16, RAGE
  ADPCM and MP3, are rows of it, and the app's `.awc` panel and `.bik` preview use it.
- **Bink 1 and XMA are ported; Bink 2's picture stays behind the table for now.** A Bink 1 movie,
  picture and sound, is decoded by `or-media`'s port of FFmpeg's decoders, Legacy's XMA `.awc`
  streams by its port of FFmpeg's WMA Pro decoder, and no FFmpeg is run, found or set anywhere. A
  Bink 2 movie's sound is a Bink audio track that table decodes, but its picture is still decoded by
  the DLL of the user's install, a row the app adds to the table, until a decoder in `or-media`
  replaces it.
- **Sound plays on every platform.** `rodio` is a dependency of every target, `ci.yml`'s Linux job
  installs `libasound2-dev`, and the null sink stays for tests.

Superseded in part on 2026-10-06 by [the filter decision](#the-bink-2-in-loop-filter-is-derived-from-rads-dll-2026-10-06): `or-media` decodes a Bink 2 picture too, so the DLL is only a Windows test's reference and no row of the app loads it.

### The corpus stages the recordings (2026-10-03)

The owner's answer with the W13 wave to the roadmap's judgement call on
[R33c](Roadmap.md#r33c-recordings-yvr-and-ywr), whose Done-when says "against the corpus" while
`openrage-corpus` staged no `.yvr` or `.ywr`: "stage the files in openrage-corpus". The Done-when
keeps its wording. `openrage-corpus/or-asset` holds a small sample of each, short and long and
the odd cases, extracted from both installs, the Legacy files flat and the Enhanced ones under
`gen9/` as the [corpus layout](../../crates/io/or-io-verify/tests/fixtures/README.md) has it, so the
two `declared_gates!` suites run on a corpus run. No testimony is recorded for either format.

### The natives database is downloaded on request, never bundled (2026-10-03)

The owner asked to "work on the natives database", finding "the most reliable source to have the
best ysc decompiler" among the community lists and the hash crossmaps maintained on GitHub. The
choice was measured over every retail script of both installs
([R63](Roadmap.md#r63-the-script-disassembler-and-the-natives-database)): alloc8or's lists name a
version 10 script's natives directly, and a version 12 script's only through a crossmap, of
which CSY0N's names the most, and adding TupoyeMenu's, the next best, names none more. The
database takes
[R40a](Roadmap.md#r40a-the-shader-definition-database-as-data)'s shape:

- **Provenance.** `alloc8or/gta5-nativedb-data`'s `natives.json` and `natives_gen9.json`, which
  state a name, parameters and a return type per native and carry no licence, and the crossmap
  pairing each native's first hash with its current one in `CSY0N/GTA-V-Crossmaps`'s
  `Enhanced Crossmap.h`, which carries none either.
- **Shipping rule.** OpenRage bundles none of it, and a test never fetches it. The `.ysc` inspector's
  button and `openrage-cli script download-natives` fetch the three files from those repositories'
  default branches into the configuration directory, only when the user asks. A list the user
  supplies with `--natives-db` or `OPENRAGE_NATIVES` still comes first.
- **Version rule.** A download records each file's upstream commit in `source.json` beside the
  files, and every output that names a native names the list and commit that named it. Downloading
  again replaces the files with the branches' current commits; nothing updates them unasked.

### A drawable whose counts contradict its runs is repaired only when asked, and only losslessly (2026-10-03)

The owner's answer to the roadmap's judgement call on the mod `.ydd` files that state no primitives,
in his words: "for the .ydd files we should repair files when the user asked for openrage
canonicalization on save and when the repair is lossless". A save that would rewrite a count a
geometry's file stated against the run it describes (`primitive_count`, `vertices_count` or
`indices_count`) is refused by default, naming the geometry and the stated and actual counts;
`or_io::decl::Repair` lets a caller choose, through `File::to_bytes_repairing`, and `to_bytes`
refuses. Asked, the save rewrites a count only where that is lossless: a triangle list's
`primitive_count` from its indices, and `vertices_count` from the vertex buffer the geometry's
vertices are; an `indices_count` that disagrees with its index buffer is two statements of one run,
and rewriting it changes which indices the game draws, so it is refused either way. A count stated
in agreement with its run that an edit to the run moves is not a repair. In the app the consent is
[the export prompt](#an-export-canonicalises-and-asks-before-it-alters-the-bytes-2026-10-01), on
export and on a loose save of an archive entry, or the setting that applies the fixes without
asking; each repair is named in the log, and in a toast when an archive entry is saved. The owner
also authorised staging such files in `openrage-corpus`, in its `mods/` directory and never in the
shared `or-asset/`, whose suites assume shipped files (2026-10-04); only the repair tests read it.

### The UX review's plan is the app's next work (2026-10-03)

The owner asked for a review of the app's UX against Blender, VS Code, Windows Explorer and the
modding tools OpenRage replaces, deciding what is missing, what to rework and what to improve, and
on reading it answered "Yeah land it, then work on the plan". The screenshot mode the review was
made with landed as [R59g](Roadmap.md#r59g-screens-an-agent-can-see) (#453), and the review's
decisions are roadmap rows: R20b, R24b, R24c, R25d to R25g, R26b, R27b, R27c, R31b, R38b, R42b,
R49e, R51d, R55b, R59h to R59q and R60c to R60e. They are worked in this order: trust first (R60d,
R25e, R20b, R59i, R27b, R59h, R59n); the front door (R60c, R25g, R59m, R59o); navigation, with the
command registry before what renders from it (R59j, then R24b, R59k, R59l); the browser (R25d,
R24c, R60e, R59p); editing (R27c, R31b, R25f); the workflows in the app (R38b, R55b, R51d, R26b,
R42b, R49e); and accessibility (R59q).

None of it reopens a ruling. A workspace stays a layout preset and a camera stays per document
([2026-09-23](#each-document-keeps-its-own-camera-a-workspace-stays-a-layout-preset-2026-09-23)),
the particle editor stays in the app as one workspace that is no longer the default (2026-10-01),
install-wide jobs stay in `openrage-cli bulk` (R21), the world editor stays a plugin (R47), and no
"never ask again" is persisted (R25). The options the review left open wait for the owner: a
sidebar switcher for Files, Search, Outline and plugins; documents side by side; a table view for
metadata arrays; a view of the mods overlay against the base game; keymap presets; a viewport in a
window of its own; "copy as CLI command"; a linked structure and hex view; and a theme drawn from
the logo's colours.

### A Bink 2 .bik is decoded by the bink2w64.dll of the user's own install (2026-10-03)

The owner reported that `update/update.rpf`'s `x64/movies/rockstar_logos.bik` showed only FFmpeg's
refusal and asked to "implement or find a way to open bink 2 or any .bik possible". No open-source
Bink 2 decoder is usable: FFmpeg decodes only Bink 1, and NihAV's Bink 2 decoder is AGPL-3.0, which
an MIT app cannot link. Both GTA V installs ship RAD's `bink2w64.dll` beside the game executable, so
on Windows OpenRage decodes a Bink 2 movie by loading the DLL of the user's own install at run time,
the same principle as the keys extracted from the user's own executable: the DLL is never copied,
bundled, redistributed or committed, and OpenRage writes no Bink 2 decoder of its own. Bink 1
stayed on the user's FFmpeg, as [the 2026-10-01 entry](#a-bik-is-decoded-by-an-ffmpeg-the-user-installs-2026-10-01)
set out, until the 2026-10-05 decision replaced that. Without an install, or when its DLL does not load or refuses the file, the preview shows
the header fields OpenRage reads and says why it cannot play. The licensing position is in
[Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources).

Superseded in part on 2026-10-05 by [the media decision](#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05): NihAV's AGPL-3.0 decoder is no longer out, and OpenRage may write a Bink decoder of its own; the DLL stays behind `or-media`'s interface until one replaces it.

### A .ysc decompiles to source (2026-10-03)

The owner's decision with the W12 wave, which reverses the roadmap's non-goal "Script
decompilation (U-137)": a `.ysc` opens in the text workspace as decompiled source, a much better
reconstruction than the instruction listing it opened as. The decompiler is ordinary code over the
script reader in `crates/io/or-io/src/gta5/script/decompile/`, because it needs only the bytes;
`openrage-cli script decompile` prints the same text, and the listing stays in the inspector and
in `script disasm`. Native names still come only from the list the user supplies, and the public
script decompilers it took inspiration from carry no licence, so nothing of theirs was copied
([Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources)).
[R63](Roadmap.md#r63-the-script-disassembler-and-the-natives-database) owns U-137 from here.

### The app plays an .awc through rodio (2026-10-02)

The owner approved an audio-output dependency for `.awc` playback with the W10 wave of format gaps.
The app takes `rodio` 0.22 (MIT OR Apache-2.0), which plays through `cpal`, with only its
`playback` feature: `or-io` decodes every stream, so none of rodio's decoders is built. It is a
Windows-only dependency, because on Linux `cpal` needs the ALSA headers the hosted CI image lacks;
a Linux build lists, exports and replaces streams and says it plays none. The output
device opens at the first play, so drawing and tests open none, and playback sits behind the
`AudioSink` trait in `src/audio.rs`, which the state tests fill with a sink that records what it
was handed. Like every crate dependency it carries its own licence and is not listed in
[LICENSE-third-party.md](../../LICENSE-third-party.md).

Superseded in part on 2026-10-05 by [the media decision](#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05): `rodio` is a dependency of every target, and the hosted Linux job installs `libasound2-dev`.

### A map no imap group names takes its static bounds by filename (2026-10-02)

The owner's answer on [R26](Roadmap.md#r26-index-dependencies-and-summaries)'s static bounds,
choosing option b: "I guess B". Only `_manifest.ymf`'s `MapDataGroups` names a map's `.ybn`, and
only for an imap group, and box overlap cannot say which map a bound belongs to: it reproduced the
manifest's answer for 0 of 388 Legacy and 0 of 380 Enhanced groups, a bound overlapping about 400
maps on average. So a map no group names takes the `<map>_N`, `hi@<map>_N` and `ma@<map>_N` `.ybn`
its stem names, reported as a filename convention and never as a stated reference, and each is
checked against the map's streaming box: one outside it is still reported, and flagged. A group map
keeps resolving through the manifest.

### R37 builds no schema-mapping framework (2026-10-02)

The owner's decision. R37 asked for "a declarative schema-mapping framework for defining new
document schemas", and it is not built: every metadata file carries its own type table, from which
any type opens, edits and round-trips with no snapshot (#397); the snapshot a user builds from the
PSC files is the user-supplied names list; and a user who wants other names edits the exported XML
and imports it back. In the owner's words, "so we dont care i guess".
[R37](Roadmap.md#r37-metadata-and-xml-at-parity) closes on the rest of its Done-when.

### Old revisions keep the engine's names (2026-10-01) (D12)

D12, taken by the owner, answering what was left to the history rewrite on 2026-09-24. When the history is rewritten, old revisions keep the game engine's names, in the owner's words "not codewalker names and not neutral names". [R66g](Roadmap.md#r66g-rewrite-the-history) renames no identifier in an old revision: it removes and redacts what [the specification](OR_IO_Specification.md#rewriting-the-history) lists and nothing else. This supersedes D12's intent of 2026-09-24 to neutralise more of the types then. When the rewrite runs is for later, below.

### The runner's stale incremental state is cleaned by hand (2026-10-01)

The owner's choice. The self-hosted runner's link failures in `main-tip` and `layouts`, where `rust-lld` reports an undefined symbol ending in `.llvm.` and a hash, come from stale incremental state in its persistent target directories, `_cargo-target` and `_cargo-target-tip`, which build incrementally because `.cargo/config.toml` turns incremental on; a clean build of the same step links. Incremental stays on. When no job is running, delete `debug\incremental` in both directories and rerun the failed job ([Development_Workflow.md](../guides/Development_Workflow.md#ci)). `CARGO_BUILD_JOBS: 4` stays on every self-hosted job, for the same error at the default `-j`. Turning incremental off on the runner was not taken.

### openrage-packs is deleted, and the .orpack export goes (2026-10-01)

The owner archived `openrage-packs` as a private, offline git bundle and deleted it on 2026-10-01, which finishes [R66i](Roadmap.md#r66i-retire-openrage-packs) as [D17](#openrage-packs-is-retired-2026-09-26) set it out; R66h now waits on R66g and the lawyer questions. Of the pack format left behind, in the owner's words, "pack formats have been deleted purposely": [R54c](Roadmap.md#r54c-the-orpack-format-and-its-compatibility-rule), a compatibility rule for `.orpack`, closes as obsolete, and asked whether the leftover `.orpack` export goes, he answered "yes". That supersedes [OD-OTHER-TITLES](#no-runtime-pack-for-another-title-2026-09-26)'s "The `.orpack` export stays" and D17's "`.orpack` stays as an export".

### A .bik is decoded by an FFmpeg the user installs (2026-10-01)

The owner wants [R34c](Roadmap.md#r34c-video-preview) ("We need R34C"), so it is not declined. Its licensing position, recorded in [Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md#sources): a `.bik` is decoded by FFmpeg's LGPL Bink decoder, run as a separate program the user installs, found on the PATH or set in the preferences. FFmpeg is never bundled or statically linked, and OpenRage writes no decoder of its own. This replaces the drafted route of linking `libavcodec` dynamically behind a feature flag. R34c stays open until the app previews a `.bik` that way.

Superseded on 2026-10-05 by [the media decision](#every-media-format-is-decoded-in-rust-inside-the-tree-2026-10-05): OpenRage ports a decoder of its own, and for Bink 1 it has, so no FFmpeg is run, found on the PATH or set in the preferences.

### No provenance record is kept, and the provenance lint stays (2026-10-01)

The owner's ruling on [R3](Roadmap.md#r3-licence-and-provenance-policy), in his words: "no code provenance info block or anything, delete it". The provenance grammar in [Licence_And_Provenance_Policy.md](../guides/Licence_And_Provenance_Policy.md), with its tags, and every `Provenance:` block in code go. The provenance lint, `scripts/lint_provenance.ps1`, stays: it still fails on a leaked-source citation in any tracked file and on committed game content. R3 stays closed. This supersedes the 2026-09-20 entry's "R3 shrinks to a light provenance record".

### Earlier RAGE titles are a future goal (2026-10-01)

The owner's ruling: GTA IV, EFLC, Max Payne 3 and RDR1 are future goals, not non-goals. They leave the roadmap's [non-goals](Roadmap.md#non-goals) and take an open, unscheduled row in Phase 9, [R57b](Roadmap.md#r57b-earlier-rage-titles), over their containers, RPF2, RPF3, RPF4 and RPF6.

### An export canonicalises, and asks before it alters the bytes (2026-10-01)

The owner's rule for [R28](Roadmap.md#r28-authoring-and-write-back-targets)'s passthrough clause, in his words: "an untouched file dont export identical bytes if you click export it will export a canonicalized version of the file so that it stays byte identical later if re-imported and exported again but if its a gta file or another tool file it will probably alter the bytes, we should have a small popup on export when the canonicalization can alter the bytes like 'Do you want to apply OpenRage export fixs (this will alter the bytes)'". An export writes OpenRage's canonical form, which a second export reproduces byte for byte. Where canonicalising changes the bytes of the file as opened, the export asks first, and declining exports the original bytes. This replaces the reading that an untouched file exports its original bytes unasked.

### A metadata name the build cannot resolve is spelled as its hash (2026-10-01)

The owner's ruling on [R37](Roadmap.md#r37-metadata-and-xml-at-parity)'s clean clone: no schema snapshot is shipped and none is needed. A name the build cannot resolve is spelled `hash_XXXXXXXX`, its hash in hex, everywhere (a field, a type, an enum value, a hashed string), in the editor and in exported XML, and import reads any `hash_` followed by valid hex back to that hash. The structure import needs comes from the documents' own type tables. The snapshot built from the PSC stays an optional extra that adds real names, found as [OD-SCHEMA-LOC](#the-editor-and-the-cli-find-the-schema-snapshot-by-a-fixed-order-2026-09-26) orders it. This answers whether a redistributable snapshot can exist: none is needed. D5's rule that no schema is bundled stands.

### Scenario regions take typed roots by hand (2026-10-01)

The owner extends [OD-META-TYPED](#world-metadata-roots-may-be-declared-by-hand-2026-09-26) to scenario regions: `CScenarioPointRegion` may be declared by hand in or_io::meta, matched against the document's own type table as the world family's roots are, for [R33d](Roadmap.md#r33d-scenario-points). D5's rule that no schema is bundled still holds.

### The roadmap's other judgement calls (2026-10-01)

The owner's answers to the rest of the roadmap's [judgement calls](Roadmap.md#appendix-b--judgement-calls-to-confirm), each now out of that list.

- The particle editor stays in the app. Parity with RPFKit is scored against its announced roadmap. Item granularity stays uneven.
- [R21](Roadmap.md#r21-extraction-and-bulk-jobs) closes: extraction from an open archive answers it, and install-wide jobs stay in `openrage-cli bulk`.
- [R33a](Roadmap.md#r33a-path-nodes-ynd) closes: `.ynd` opens as a `DocumentKind` of its own with an outline and a save (#351). [R33b](Roadmap.md#r33b-navmesh-ynv) stays open for a 3D view the owner judges.
- The ticks on [R19](Roadmap.md#r19-one-virtual-filesystem) and [R55](Roadmap.md#r55-retargeting-between-gen8-and-gen9) stand.
- [R67](Roadmap.md#r67-menyoo-spooner-xml-import) waits for a corpus of spooner files, unscheduled.
- R66g's rewrite is for later, and R66h and the lawyer questions (D14) stay parked.

### The plugin host, the registries and the second-title seams stay (2026-09-30)

The owner's ruling (OD-AUDIT-KEEPS), after the over-engineering audit was re-checked item by item. `or-plugin` and the app's tiered registries stay, for [Phase 7](Roadmap.md#phase-7--plugins-and-open-with) ([R44e](Roadmap.md#r44e-consent-and-refusal-in-the-app) to [R47](Roadmap.md#r47-the-plugin-slate-after-the-first)), and so do `PagingModel`, `ResourceEnvelope` and every `GameProfile` method, for [R56](Roadmap.md#r56-rdr2-the-second-container)'s RDR2. That they have one caller today is not a reason to delete them: a dead-code sweep leaves them, and narrowing them is not taken either.

### The testimony key is frozen, and its recipe goes with the pack (2026-09-30)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-TESTIMONY-KEY-FROZEN), and checked against `main` at `eb500483`. The testimony's frozen key, `pack-witnessed.key` beside the testimony, was written once from the version 12 pack, and every declared format's testimony check and `the_recorded_testimony_holds` read it. P62 deletes GTA V pack discovery with `or-asset`, so the recipe that wrote the key, just testimony-key, and its recorder test, which read the pack through discovery, can no longer run and are deleted with it: the key is never retaken, and nothing writes into `openrage-corpus/testimony` any more. A key that is missing is a failure that names this, not a prompt to run a recipe. Keeping the recipe with nothing it can read was not taken.

### The pack tooling goes before pack discovery (2026-09-30)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-P61-SPLIT), and checked against `main` at `c7ef57b6`. [D17](#openrage-packs-is-retired-2026-09-26) deletes the pack recipes and GTA V pack discovery, but `or-asset`'s corpus suites still read a pack through discovery until R66f deletes that crate. So the deletion is split. P61 deletes the tooling: `just pack`, `pack-test`, `pack-check` and `pack-sync`, the script that found the pack repository and its cargo profile, the `tools/` rewriters that threaded packs through `or-asset`, the export's provenance stamp and its contract paths, `or-io-verify`'s `pack-sync` binary and the test that held the two repositories in step, and the `layouts` job's pack staging and gate-6 step. `just pack-check` ran for the last time at `3e822330` and exited 0, and the merge rule's `just pack-check` clause lapses with the recipe. Pack discovery (`or_io::desc::discover`, `OPENRAGE_PACKS` and the sibling walk) stays, test-only, reading the `dist/` frozen at that last export, which `layouts` names through `OPENRAGE_PACKS`; P62 deletes it with `or-asset`, and `just test-no-pack` keeps its three variables until then. Deleting discovery now, which turns those suites into skips, was not taken.

### A baseline recorded from a wrong reader is corrected by name (2026-09-30)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-P60-CORRECTIONS), and checked against `main` at `243181d5`. The CLI record ([OD-BASELINES](#r66es-behaviour-baselines-live-in-openrage-corpus-2026-09-26)) taken at `29722735` holds the compiled reader's output for `inspect skeleton` on `des_bridge_root.ydr` and `des_fib_ceil2_end.ydr`, which printed `Bones: 0` because that reader dropped their skeletons; P60's owned read prints the 23 and 4 bones the files carry. The baseline comparer gains a `CORRECTED` list beside `NAMED_REFUSALS`, as the testimony's does: each entry names the file and command, the line the record holds, the line that is right and why, and a listed entry whose output does not print the right line in place of the recorded one fails the run. Printing the old wrong output on purpose, and merging with the comparer red, were not taken.

### A vertex buffer's stride is derived at save (2026-09-30)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-VB-STRIDE), and checked against `main` at `9359a0fe`. A stride is a derived value under [OD-RUNTIME](#an-owned-value-shows-no-padding-count-or-pointer-2026-09-26), and a vertex buffer built from scratch could not state its private one, so its data's product length refused every save. The framework gains `#[derived(f)]` on a private field: a save writes `f(&self)` with the other fields, before the count fixups that divide by it, and a value built from scratch saves what it derives. `GrcVertexBuffer` derives its stride from its FVF's `fvf_size`, `GrcVertexBufferG9` from its declaration's vertex size, and `GrmGeometry` its `vertex_stride` from its buffer's; all 200 strides in the staged `.ydr` agree (`every_shipped_stride_is_what_a_save_derives`). A stated value that disagrees with its derivation is never silently rewritten: it saves as the file stated it while the derivation is what it was at open, and is refused once an edit moves the derivation. P50 landed it as a framework package. A public stride field, and a recompute hook run before the count fixups for every record, were not taken.

### Interchange moves after the owned fragment, and takes the CLI's calls with it (2026-09-29)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-P48-SEQ), and checked against `main` at `651b5e75`. P48 moves `or-interchange` onto the owned drawable after P43 has made `.yft` owned, so the export, the import and their suites move once for `.ydr`, `.ydd` and `.yft` together. The same change rewrites the `or-interchange` calls in `crates/tools/or-cli/src/mesh.rs`, because a re-export shim is ruled out. Moving `.ydr` and `.ydd` first and `.yft` later, and a shim that keeps the old signatures for the CLI, were not taken.

### A pointer the file aliases keeps its declared type (2026-09-29)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-ALIAS-BY-VALUE), and checked against `main` at `ea1b6c8c`. A pointer a file aims at a target another pointer owns elsewhere in the tree keeps the type its format declares (`Option<Box<T>>`, an element of `Vec<Option<Box<T>>>`, `String`), so the engine-named twin's `if let Some(skel) = &car.skeleton` compiles unchanged. It reads as a copy of the owner's value, and the hidden carry records which path owns it; a save writes the alias while the copy still equals the owner, and a separate allocation once an edit makes them differ. It is lossless: an untouched value saves byte for byte as its tree does, and an alias no pointer owns is refused at open. The `.yft` lane measured the shapes with `Format::read` over the corpus (Legacy / Enhanced): a drawable's skeleton aliasing the common drawable's (817 / 73), a composite's child bound aliasing the matching child drawable's bound (798 / 72), a cloth's referenced drawable aliasing the common drawable (16 / 1), a physics level's composite bounds aliasing the undamaged archetype's bound (64 / 5), an archetype's name aliasing another's (9 / 2), and a physics level's group names pointing into each group's own name (64 / 5), which the tree already reads as strings of their own. [OD-REFS](#a-pointer-at-what-another-pointer-owns-is-never-a-public-address-2026-09-26)'s `Ref<P>` stays for the `.ypt` lists of run elements, where a reference is the natural owned API. A `Ref<P>` at each of these fields, which moves the twin line behind a match, and a save that un-aliases silently were not taken.

Extended on 2026-10-02 to runs, after a modded `.ydd` the owner opened was refused because one geometry's `matrix_palette` aims at the run another geometry's owns: a `Vec<T>` field, counted or `#[len]`, whose pointer aims at a run another pointer owns reads as a copy of the owner's run and saves under the same rule, as the alias while the copy equals the owner and as a run of its own once an edit makes them differ. An aliased counted array states its owner's count and capacity, as P83 (#307) made the canonical writer do. An alias no pointer owns is still refused at open (`a_run_alias_no_pointer_owns_is_refused_at_open`).

### A texture's pixel length is implied from its mip chain before the packs retire (2026-09-29)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-IMPLIED-PIXEL-LEN), and checked against `main` at `40aeb6bd`. A Legacy texture whose `physical_size_and_template` states a length of zero beside a live pixel pointer is read at the length its format, dimensions and mip count imply, through a new `CountSource` kind, `MaskedOrMipChain`, which [OD-PACKVER](#pack_version-stays-12-until-the-pack-is-deleted-2026-09-26) allows in the built-in descriptions with no `.orpack` extension and no bump, so it did not wait for the packs to retire; P80 did not take it. It landed with P81: `GrcTexture`'s `backing_store` takes `#[len(masked_or_mip_chain = ...)]`, a save leaves the word at zero while the run is the length the mip chain implies and states the length otherwise, and the five shipped files it refused (the Legacy `des_bridge_root.ydr`, `des_fib_ceil2_root.ydr`, `des_smash_end.ydr` and `des_smash_root_merge.ydr`, and `server_assets.ytd`) open owned and save as their tree saves, at a fixed point, with no owned-path set-aside naming them. The descriptor pack path, which cannot carry the kind, still refuses `server_assets.ytd`. A texture whose word and mip chain both state nothing is still refused. Bumping `PACK_VERSION` for the new kind was not taken.

### A Legacy texture's RGBA is checked against the compiled reader, then recorded as a digest (2026-09-28)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-TEXTURE-DIGESTS). P46's done-when, every corpus Gen8 texture decoding to a recorded RGBA digest, is met while `or-asset` stands by `every_corpus_legacy_texture_decodes_as_the_compiled_reader_decodes_it`, which opens each corpus Legacy `.ytd` owned with no pack and compares every texture's full RGBA bytes and sRGB flag with what the compiled reader decodes: 288 textures in 15 of the 16 corpus files, all equal. The sixteenth, `server_assets.ytd`, is refused by the owned open, because one texture's pixel pointer is live beside a stated length of zero ([OR_IO_Specification.md](OR_IO_Specification.md)); `main`'s compiled reader read it with a pack. [OD-BASELINES](#r66es-behaviour-baselines-live-in-openrage-corpus-2026-09-26) widens to per-texture RGBA digests, digests only and written by `just baselines-record`, which are recorded before P62 deletes `or-asset`, so the check outlives its oracle.

### The install testimony sets aside what the declarations cannot yet read (2026-09-28)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-P45-SETASIDES), and checked against `main` at `395de759`. `the_install_testimony_holds` names two sets in its `REFUSED` list, each file by its install path with its reason, and a listed file that reads fails the run. The 42 retail Enhanced `.ypt` files whose behaviour's `keyed_values` points at a `KeyedValue` member of its `Behaviour` and lands outside it, as the Gen9 `Behaviour` is declared, are set aside until the Gen9 `Behaviour` declarations read them, which removes them from the list. The Legacy install's `mods_/ped_edits/eye/mp_eye_colour.ytd`, a user's mod whose texture states 0x2b400 payload bytes on 0x20000 of physical pages, stays set aside, because the declared read rightly refuses it. Failing the run on either set, and relaxing the declared read to accept the mod, were not taken. Since then the Enhanced `.ypt` files left the list with their declaration, and the declared read takes the mod: the byte-identical file read through `desc::value::read` on 2026-10-09, so wave P11 removed its entry, which a listed file that reads would otherwise fail. The decision's sentence that the declared read rightly refuses the mod no longer describes the tree; when the read came to accept it was not traced.

### A null level of detail reads as an empty one (2026-09-28)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-TWIN-LODS), and checked against `main` at `c6057e15`. The engine-named twin keeps car.lod_group.lods[0].models as written, though the game leaves level-of-detail slots null (lods[1..3] in 13 of 13 staged Legacy `.ydr`, lods[2..3] in 5 of 5 Enhanced) and D16 allows `Option` only where the game allows null. The level-of-detail slots take the framework's `NullIsEmpty` pointer: a null slot reads as an empty level of detail, its type's default, and a save writes a level equal to that default back as null. It is lossless: a slot a file points at an empty level is kept live, and no staged file has one (no non-null level holds zero models). `Option` slots, which move the twin line behind a match, and rewriting the twin line were not taken.

### A vertex buffer's count is read through an accessor (2026-09-28)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-TWIN-VCOUNT), and checked against `main` at `c6057e15`. The engine-named twin's geom.vertex_buffer.vertices_count becomes geom.vertex_buffer.vertex_count(), an accessor over the vertex data, because the count is an element count and stays private under [OD-RUNTIME](#an-owned-value-shows-no-padding-count-or-pointer-2026-09-26). A public count field, which the refusal of a public count forbids, was not taken.

### or-io ships the key snapshot where archives are opened (2026-09-28) (D18)

D18, the owner's own. In the owner's words: "its a key that needs a pattern of the game executable of user to get it so thats fine we should ship it", and on where: "We should put it in what opens rpf i think". CodeWalker's key snapshot, which carries the NG keys no retail executable does, is masked and encrypted under the game's AES key, so it decodes only with the key extracted from the user's own executable. `or-io` ships it as `crates/io/or-io/data/magic.dat`, embedded in `container::rpf::crypto` behind the default-on `bundled-keys` feature, which a consumer can turn off. A snapshot of the user's still comes first (their `magic.dat` beside the keys cache for the app and `openrage-cli`, `OPENRAGE_KEY_SNAPSHOT` for the library's default path), and one that does not decode or describes another build falls back to the bundled one; the keys cache stays per user and per executable. The published crate will carry the snapshot permanently once P7 publishes it, which the owner accepts. Lawyer question 4 stays open and still gates P7 (D14). This supersedes D4's "no keys ship" for the key snapshot, R66b's removal of it from the tree, and R66g's removal of it from history.

### Agent limits are off until the owner starts them (2026-09-28)

Agent work runs with no CPU cap and no limits by default. `just limits-start` turns the watcher on, with the saved `just limits-cap` value, until `just limits-stop` or a restart; the `OpenRage agent limits` task has no logon trigger.

### The local checks are one recipe (2026-09-28)

The merge rule's checks are `just ready` (clippy, `just self-check` when anything under `src/` changed, and `cargo test --bin openrage`), or `just ready-full` (the same with `just test`) when a format's read or write path changed, with `just pack-check` while that clause stands. The format check, the encoding lint and the provenance lint are the pre-commit hook's and are not run again, and feature combinations are left to `main-tip.yml`. `just ci` and `just self-check-no-pack` are removed.

Superseded on 2026-09-30 by [the split](#the-pack-tooling-goes-before-pack-discovery-2026-09-30): the `just pack-check` clause lapsed when the recipe was deleted.

Superseded on 2026-11-20 by [the scoped gate](#an-agent-tests-what-its-change-touches-and-a-default-run-takes-thirty-seconds-2026-11-20): `just ready` checks what the change touches, `just ready-full` no longer runs `just test`, and the separate self-check step is gone.

### R66's open decisions take the plan's recommendation (2026-09-26)

The owner's standing instruction for R66. Asked what a package should do when it reaches an open owner decision, he chose "Take your recommendations": the plan's recommendation is taken as his ruling and recorded as a dated entry saying it was approved under this instruction, and work stops only where the tree contradicts the recommendation or no option is safe. Each entry below that says it was taken under this instruction was checked against `main` at `d61be5b6` before it was recorded. [The stale lanes](#the-stale-lanes-are-preserved-then-removed-2026-09-26) are the owner's own choice, not a recommendation.

### Drawables are one generic declaration per class (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-10.8). It answers the specification's question 10.8 and amends D16. A drawable is one generic declaration per engine class, `GtaDrawable` generic over `DrawableTarget` with `Gen8` as the default, as the raw declarations already are, so no field holds a target. GtaDrawable::open opens a Gen8 drawable and refuses a Gen9 one by name, pointing at or_io::open, and only the refusal a user reads says Enhanced; a caller that does not know the target matches or_io::open's `Asset` for either drawable variant; GtaDrawable::\<Gen9\>::open is the typed Gen9 call. The target is the file's RSC7 version, so a Gen8 drawable from an Enhanced install opens through GtaDrawable::open. Field access is unchanged from D16's sample. The recommendation spelled the Gen9 variant Asset::DrawableEnhanced; that identifier is not ruled here, because code names a target Gen8 or Gen9, and the package that builds `Asset` names it under that rule. An owned enum per root was not taken, because it moves every field read of the sample behind a match; one root with per-target leaves is ruled out, because a Gen8 struct never holds Gen9 data and the four types Gen9 changed are declared per target.

### Gen9-only words take measured names (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-10.9). It answers the specification's question 10.9. A Gen9-only word the GTA source does not declare, in the Gen9 texture, the Gen9 drawable declarations or the pack's own unnamed Gen9 fields, takes a measured name, as the Gen9 texture's `block_count` and `block_stride` already do, and a Direct3D structure embedded in a resource, such as the shader resource view, takes Direct3D's names. A word is carried as bytes only case by case with a recorded reason, never as padding. A named byte array for every unmeasured word, a loophole in D15, and keeping Gen9 raw until a source exists were not taken.

### The pose database is ordinary code (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-YPDB). It answers the pose database's scope, left open on 2026-09-24. `.ypdb` moves into `or-io` as ordinary code in gta5::pose_db, with no `#[record]` declaration, and opens and saves as an owned value (D16) through the file trait. It is one serialized stream with no paging or pointers and not an RSC7 resource, so the rule that every RSC7 format is declared does not reach it. Declaring its fixed parts, with the trailer words named or carried, was not taken.

### `.ysc` is ordinary code until a corpus exists (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-YSC). A dated, time-limited exception to the rule that every RSC7 format is declared in `or-io` (ruling 3 of 2026-09-24, and D1): `or-asset`'s `ScrProgram` is a pack-bound `#[rage_struct]` and goes with `or-asset`, so the reader moves into `or-io` as ordinary Rust, plain structs and a hand-written reader with no `#[record]` and no pack binding. The declared form is owed once corpus files are staged. Declaring it owned now without corpus evidence, and dropping `.ysc` with `or-asset`, were not taken.

### The packed-alignment ruling stands until every format is owned (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-ALIGN). R66d's rule that a record run a canonical save places off natural alignment is declared `packed(N)` holds through R66d and R66e, so each owned migration keeps its figures, and is decided once every format is owned, from the alignment census. Revisiting it now, and keeping `packed(N)` as the permanent canonical form, were not taken.

Decided on 2026-09-30 (P68, #338), with every format owned: a canonical save places every record run at its natural alignment, and `packed(N)` stays only where a shipped layout needs it, `.ynv`'s quadtree node ([R66d](Roadmap.md#r66d-declare-gta-v-format-by-format)).

### The app holds one resource payload (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-PAYLOAD). The app's resource payload is one `DocPayload` variant, DocPayload::Resource holding or-io's Asset, introduced with R66e's first rows, and each format moves into it once. Per-format variants holding `or-io` types, collapsed later, were not taken: every row would migrate twice, which D16's migrate-once rule forbids.

### The format registry splits between or-io and the app (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-REGISTRY). `or_asset::format_registry` splits: its detection, the header and extension scoring and the target, becomes or_io::open and its Asset, and `FormatHandler` and `Tier` move into the app's `src/state/` as editor policy. Moving both halves into `gta5::formats`, where the specification's module map had put it, and keeping it as it is were not taken.

### or-ptx splits between or-io and or-particle (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-PTX). `or-io` cannot depend on `or-ptx`, so its file vocabulary, `ShaderVar` included, moves into `gta5::particle` and its editor half into `or-particle`, with behaviour unchanged (D9) and held by R66e's simulation baselines. The crate is deleted only after `or-asset` and the pack repository stop depending on it, since both import it today. Keeping the behaviour codec and `YptAsset` in `or-particle` was not taken.

### D16's sample stays as approved, and an engine-named twin compiles (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-SAMPLE). The sample the owner approved on 2026-09-25 stays verbatim as dated, though it names Ymap, Texture and ytd.textures, which are not engine names, and mixes `or-io` calls with `or-title` calls. The acceptance check compiles an engine-named twin (`CMapData`, `GrcTexture`, entries) line for line, as an `or-io` example for the file half and an `or-title` example for the install half, because `or-io` cannot depend on `or-title`, with a ratchet in `src/doc_counts.rs`. The specification already says engine names win, so this changes no decision. Rewriting the sample, and aliases, which D15 and D16 forbid, were not taken.

### World metadata roots may be declared by hand (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-META-TYPED). Typed roots for the world family of metadata (`CMapData`, `CEntityDef`, `CMapTypes` and the archetypes) may be declared by hand in or_io::meta, each matched against the document's own type table. D5's rule that no schema is bundled still holds: a document carries its own table, so a typed root is a declaration like any `#[record]`, not a bundled schema. Other metadata stays schema-driven. Dropping the sample's metadata line, and typed roots only in an unpublished crate, were not taken.

Extended on 2026-10-01 to scenario regions by [the owner](#scenario-regions-take-typed-roots-by-hand-2026-10-01), and on 2026-10-07 to the ped variation family (`CPedVariationInfo` and `CCreatureMetaData`) by the default of [the Workbench plan](Workbench_Plan.md#defaults-taken-on-the-owners-go).

### The editor and the CLI find the schema snapshot by a fixed order (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-SCHEMA-LOC). With no schema compiled in, the editor and the CLI take the metadata schema snapshot from `OPENRAGE_SCHEMA`, then from the user's configuration directory beside the keys cache, then, from a source tree only, from the ancestor openrage-corpus/or-meta/v2. A source tree still finds the shared store it finds today, but the order differs from today's: `or_meta::snapshot_discovery::search_paths` tries `or-meta`'s own copy in its data directory first and `OPENRAGE_SCHEMA` second, and the ruled order drops that copy and puts the variable first. The store stays where it is, and its standing, left open on 2026-09-24, is settled by this ruling: it stays in `openrage-corpus`. Only a path chosen in Settings, and moving the store, were not taken.

### The corpus directory keeps its name (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-CORPUS-NAME). The shared corpus directory stays openrage-corpus/or-asset after the `or-asset` crate is deleted, and only `or-test-support`'s constants change. The name is a string, and renaming it would touch the runner's workflow and every lane for no gain.

### Field-level bit ranges are ported into `#[record]` (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-BITS). Before the pack goes, `#[record]` gains a field-level `#[bits]`, and each owned migration declares its bit ranges under the source's names: `write::patch`'s `PatchError::BitPacked` refusal and the plugin host's bit-packed check rely on them. A note is carried only where a declaration needs one; the pack's notes are not bulk-copied, because they risk leak-derived text. Dropping both was not taken.

### The vehicle-glass decoder moves into gta5::fragment (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-GLASS). `or_asset::yft::vehicle_glass`, which nothing outside `or-asset` uses, moves into `gta5::fragment` with `.yft`'s lossless tables rather than being deleted. The lossless save must keep the Gen8 glass blocks anyway, and the decoder is the only test that the carried bytes still mean something.

### No runtime pack for another title (2026-09-26)

Taken under the [standing instruction](#r66s-open-decisions-take-the-plans-recommendation-2026-09-26) (OD-OTHER-TITLES). It answers what [D17](#openrage-packs-is-retired-2026-09-26) left unruled. The runtime `.orpack` the specification promised for a title `or-io` does not declare, giving the generic tree, the inspector and the canonical writer, is dropped: GTA V is the only declared title, the loader has no user, and keeping it keeps the pack-format read path alive in `or-io`. A new title gets the generic tree from its own declarations. `GameProfile::pack_title` is removed and the suites that name it, `crates/title/or-title/tests/a_title_with_its_own_target.rs` among them, are rewritten. The `.orpack` export stays. The loader can come back when a second title is declared. A runtime loader for a user-chosen pack of an undeclared title was not taken.

Superseded in part on 2026-10-01 by [the export's removal](#openrage-packs-is-deleted-and-the-orpack-export-goes-2026-10-01): the `.orpack` export goes.

### The stale lanes are preserved, then removed (2026-09-26)

The owner's own choice (OD-STALE-LANES), which differs from the plan's recommendation to discard them. Of the lanes the janitor would hold, the dirty pack-only-audit and dx-pack-access and the detached r37-schema-gaps and r64f-cli-writes-through-pack were preserved and then removed: uncommitted work was committed as a local WIP on a preserve/* branch, never a stash; a detached HEAD was given a branch; research found nowhere else was copied into docs/extra; then the worktree was removed. Two dirty worktrees whose old trees cannot run today's pre-commit hook were left in place on purpose, and the janitor keeps reporting them: the lane r37-schema-typed-probe, and agent-ae40727c07fa874ab, made under the checkout's .claude directory, whose branch is already in `main`.

### openrage-packs is retired (2026-09-26)

The owner's decision (D17), taken with the seven rulings below in his words "I agree with your recommendations on the decisions". It answers the plan whose goal he set on 2026-09-25: everything moves into `or-io`, one macro crate stays, every crate no longer needed is deleted, and `openrage-packs` is removed completely. GTA V has one description, the one `or-io` builds from its declarations.

- The proof of a layout is the typed reads, the recorded testimony, the canonical fixed point, and the alignment, padding and variant censuses. The structural diff against the private pack (gate 6) is retired format by format, in the pull request that migrates the format to the owned form, after its last output is logged.
- The testimony's pack-witnessed fields are read through a frozen offset key, below; `PACK_VERSION` stays 12.
- `.orpack` stays as an export (superseded on 2026-10-01 by [its removal](#openrage-packs-is-deleted-and-the-orpack-export-goes-2026-10-01)). GTA V pack discovery is deleted. Whether a runtime pack for a title `or-io` does not declare stays is not ruled here. Answered on 2026-09-26 by [OD-OTHER-TITLES](#no-runtime-pack-for-another-title-2026-09-26): it does not.
- The 2026-09-25 merge rule's `just pack-check` clause holds until the pull request that deletes the recipe, and then lapses.
- Once nothing reads the repository, the owner archives it as a private, offline git bundle, outside R66g's rewrite and never published, and deletes it: [R66i](Roadmap.md#r66i-retire-openrage-packs), done on 2026-10-01. R66h depends on R66f and R66i.

It supersedes the "pack conformance" clause of D15's proof, the specification's "structural diff against the private pack" clause, and R66f's and P5's plan that the pack stays as the structural diff's second description. It answers the open question of what the private pack is written in after R66f: nothing, because it is retired.

### PACK_VERSION stays 12 until the pack is deleted (2026-09-26)

The owner's ruling (OD-PACKVER), with [D17](#openrage-packs-is-retired-2026-09-26). No wire bump: one framework lane adds the vocabulary a lossless save needs to the built-in descriptions only, with no `.orpack` extension, pack branch, `just pack` or `just pack-sync`, and each format's lossless save is done inside that format's owned migration (`.ybn`, `.yft`, `.ypt`). Any bump would make the staged pack refused in every lane, and two unmerged branches each claimed version 13 for a different variant.

### The testimony's frozen key sits beside it, and recording ends (2026-09-26)

The owner's ruling (OD-TESTIMONY), with [D17](#openrage-packs-is-retired-2026-09-26).

- The exception that `openrage-corpus` holds the testimony beside game files is widened to the testimony plus its key: a frozen table of the pack-witnessed offsets, taken once from the version 12 pack and written only by a named recipe, just testimony-key, the way `just snapshot` writes the schema store. `AGENTS.md`'s lane-root exceptions name it.
- `the_recorded_testimony_holds` may move into `or-io-verify` under the same name and default path; the testimony itself does not move.
- New testimony can no longer be recorded once the compiled readers go. [R65c](Roadmap.md#r65c-record-the-oracles-testimony) is closed as done, with its three Gen9 rows (`.ybn`, `.ycd`, `.yld`) ruled not recordable, because neither install ships such a file.
- The CLI's testimony command becomes an install-gated `or-io-verify` test.

Superseded in part on 2026-09-30 by [OD-TESTIMONY-KEY-FROZEN](#the-testimony-key-is-frozen-and-its-recipe-goes-with-the-pack-2026-09-30): the key stays beside the testimony, and just testimony-key, which the first point names as its writer, is deleted with pack discovery, so nothing writes it again.

### R66e's behaviour baselines live in openrage-corpus (2026-09-26)

The owner's ruling (OD-BASELINES), with [D17](#openrage-packs-is-retired-2026-09-26). The baselines R66e is judged against (`.glb` content, poses, simulation snapshots, CLI outputs) are digests only, taken from one named `main` revision, in openrage-corpus/baselines, and written only by a named recipe, just baselines-record. That widens the 2026-09-24 corpus ruling a second time. Digests keyed by corpus paths are derived from game files, as the testimony is, and a dual-path test cannot compare the old and new versions of one consumer crate.

### An owned value shows no padding, count or pointer (2026-09-26)

The owner's ruling (OD-RUNTIME), with [D17](#openrage-packs-is-retired-2026-09-26). Padding, element counts, reference counts, vft words and runtime pointers are declared, so the layout and the description name them, but they are not public: they live in `__carry` or in private fields. Derived fields, such as the `.yfd` signature CRC, a string's length and allocation, and a dictionary's code order, are recomputed at save. A gate refuses a public padding, count or pointer field on any owned record. D15's "declared" is about the layout and the description; D16's "invisible" is about what the user sees.

### A pointer at what another pointer owns is never a public address (2026-09-26)

The owner's ruling (OD-REFS), with [D17](#openrage-packs-is-retired-2026-09-26). Such a pointer (a reference-counted reference, a composite bound's shared matrices, a texture variable's texture, an effect spawner's rule) is derived on save where it is structural, held by key where its target is keyed, and held once where the target is shared. Each shipped pointer is measured on the corpus against its derivation and carried only where it differs. A save that writes null where the file holds a pointer stays a defect.

### Parallel lanes: three compiling, five built (2026-09-26)

The owner's ruling (OD-CONCURRENCY), with [D17](#openrage-packs-is-retired-2026-09-26). At most three lanes compile at once and at most five lanes holding a build stay open, under this machine's 25% per-tree CPU cap and the all-agent memory ceiling, on 8 logical CPUs. A merged lane runs `cargo clean` straight away. The figures are re-measured with `just limits-status` after the first wave. [Development_Workflow.md](../guides/Development_Workflow.md#how-many-lanes-at-once) holds the rules.

### The in-flight R66d lanes are redone under new names (2026-09-26)

The owner's ruling (OD-LANES), with [D17](#openrage-packs-is-retired-2026-09-26). A rework cannot reuse its lane's name, because `just agent-worktree` fails on an existing branch, and a branch whose work has not landed is kept until its replacement lands.

- r66d-rpf7 (#217) is redone in r66d-rpf7-d15 as a new pull request; #217 is closed as superseded, with a link, and the old lane is removed on the owner's word after that. No new work lands under #217's number.
- r66d-gxt2 is redone in r66d-gxt2-d15, salvaging its parser and tests, with no re-export shim.
- r66d-ypdb is redone in r66d-pose-db once the pose database's scope is ruled.
- r66d-ysc stays parked, and its `PACK_VERSION` bump is dropped.
- r66d-owned2 commits its uncommitted work locally, never as a stash, and stops; later lanes salvage it.
- agent/r66d-lossless is abandoned as a branch once later lanes have salvaged it.

### AGENTS.md holds rules and links, and superseded documents are deleted (2026-09-25)

`AGENTS.md` holds the rules every session obeys and links to the documents that own the detail ([Documentation_Standard.md](../guides/Documentation_Standard.md)), and adopts the Karpathy guidelines, adapted, as its [working principles](../../AGENTS.md#working-principles): think before coding, simplicity first, surgical changes, goal-driven execution. A superseded document is deleted once what is still needed has moved to the owner of its topic, rather than kept as a record under a status line:

| Deleted | Content now in |
|---|---|
| No_Rage_Structs.md | [OR_IO_Specification.md](OR_IO_Specification.md) |
| Workplan_Format_Coverage.md | [Roadmap.md](Roadmap.md) (deferred work and known gaps), [New_Format_Implementation_Guide.md](../guides/New_Format_Implementation_Guide.md) |
| Branch_Reconciliation.md, Coordinator_Handoff.md, v2/README.md | [Roadmap.md](Roadmap.md) |
| Coordinator_Skill.md, v2/dispatch.md | [Development_Workflow.md](../guides/Development_Workflow.md) |
| What_Replaces_The_Compiled_Oracle.md | [Oracle_Testimony.md](../guides/Oracle_Testimony.md) |
| v2/01-layout-authority.md, v2/02-pack-write-path.md, v2/03-retiring-compiled-readers.md, v2/decision-interior-structure.md | Descriptor_Packs.md, itself deleted on 2026-09-30 (P67), its description model and `.orpack` export in [OR_IO_Specification.md](OR_IO_Specification.md#descriptions) |
| v2/04-gen9-collapse.md, v2/05-format-tax.md, v2/06-the-shell.md, v2/07-titles-and-plugins.md, v2/09-container-seam.md | [Crate_Reference.md](../reference/Crate_Reference.md) |
| v2/08-proof.md | [Documentation_Standard.md](../guides/Documentation_Standard.md) |

### Landed branches are deleted (2026-09-25)

Once a pull request's work is on `main`, its branch is deleted on `origin` (the merge's `--delete-branch`) and locally (`git branch -D`, once the janitor has removed its worktree). A branch whose work has not landed is never deleted, and `just janitor` still removes only worktrees. This reverses the rule that branches are never deleted; [Development_Workflow.md](../guides/Development_Workflow.md) has the procedure.

### A pull request merges once the local checks pass (2026-09-25)

When `just fmt`, `just lint`, `cargo test --bin openrage` and `just self-check` pass (with `just test` when a format's read or write path changed, and `just pack-check` when a layout or the pack format did), push once, open the pull request and squash-merge it straight away. Hosted CI and `layouts` are read afterwards, on `main`; neither gates a merge.

Superseded on 2026-09-26 by [D17](#openrage-packs-is-retired-2026-09-26): the `just pack-check` clause holds until the pull request that deletes the recipe lands, and then lapses.

Superseded on 2026-09-28 by [one recipe](#the-local-checks-are-one-recipe-2026-09-28): the checks are `just ready` or `just ready-full`.

### A file is an owned Rust value (2026-09-25, 06:45)

D16. The owner approved the target calls: `GtaDrawable::open(path)`, fields read and assigned directly, structural edits as `Vec` operations, `save()` and `save_as()`, `or_io::open` into an `Asset` enum, `Gta5::find()` with `gta.open()` and a save to a mod folder in `or-title`, `to_png`, `from_png`, `mesh()`, and typed metadata such as `Ymap::open`. The backend is adapted to them and each format migrated once, to that shape: one declaration yields the owned struct, its reader and canonical writer, the computed raw layout, the built-in description and an opt-in zero-copy `peek`. Engine names are the API, with no aliases and no wrapper type. The rules of 05:48 and 05:54 stand.

### Declarations follow the GTA source (2026-09-25, 05:48 and 05:54)

D15. Type, field, enum and value names are the engine's, in the Rust casing `#[rage_struct]` used; field order, types, padding and inheritance follow the source, and CodeWalker, the corpus and the testimony cross-check the layout. Offsets are computed: `#[record]` lays a struct out by `repr(C)` field order and checks it tiles `size`, padding is a declared field or `#[pad_after(N)]`, and `#[at]` is an optional assertion. There is no `unknown_*` or `unk*` field, and no `#[provenance]` attribute, provenance lint rule on engine names or leak-name denylist: the testimony, pack conformance and the fixed point are the proof. Citations of the source tree by path, file or line, and code or comments translated from it, stay out of committed text. Engine names in public code are the owner's decision, taken with the risk the specification's legal section describes. This supersedes the own-names half of D1, the functional renames, D6's overlay, and R66b's provenance and denylist rules.

Superseded on 2026-09-26 by [D17](#openrage-packs-is-retired-2026-09-26): "pack conformance" is no longer part of the proof.

### OpenRage declares GTA V's structures itself, in one public crate, `or-io` (2026-09-24)

The owner's decisions of 15:02 to 15:36. They supersede the 2026-09-23 entry and rulings 1 and 4 of the pack-everywhere entry below.

- D1: "no RAGE structs" and the private pack as holder of the names are superseded; GTA V's structures are declared in public, and the pack is no longer the layout or name authority. Its own-names half superseded on 2026-09-25 by D15.
- D2: the crates are `or-io` and `or-io-macros`, taken on crates.io. Workspace crates take the `or-` prefix; the app package is `openrage`.
- D3: this repository is published, after its history is rewritten to remove comments and documents about leaked code.
- D4: keys are extracted from the user's own game executable; none ships. Its "none ships" superseded for the key snapshot on 2026-09-28 by [D18](#or-io-ships-the-key-snapshot-where-archives-are-opened-2026-09-28-d18).
- D5: metadata becomes `or_io::meta` behind a `meta` feature, with no schema bundled.
- D6: engine names are kept for now, privately, with a display overlay. Superseded on 2026-09-25 by D15.
- D7: the CodeWalker- and CodeX-derived code stays on its authors' permission, credited in `LICENSE-third-party.md`.
- D8: the crate and the editor are published at the same time.
- D9: the particle simulation ships at full fidelity to GTA V; it is the highest legal-risk item.
- D10: `or-asset` merges into `or-io` whole and is deleted.
- D11: texture authoring is `or_io::authoring` behind `authoring`, plus `bc7-encode`.
- D12: engine-named code in old revisions is decided in the history rewrite (R66g), with the intent to neutralise more of it then. Taken on 2026-10-01 by [D12](#old-revisions-keep-the-engines-names-2026-10-01-d12): old revisions keep the engine's names.
- D13: D9 extends to the renderer's fallback shaders, the VFS device code and the particle reset values.
- D14: the lawyer questions are parked and gate publication only.

Consequence: R66 is the critical path, R65's open rows are closed as kept, replaced, obsolete or withdrawn, and publication is R66h.

### The testimony stays in `openrage-corpus`, the primitives stay, and YPDB is open (2026-09-24)

The owner's corrections to how the entry below first recorded his answers.

- The R65c testimony (79 files, 13.5 GiB on 2026-09-25) stays in `openrage-corpus/testimony` as the one explicit exception to that directory holding game files: the values the compiled readers read, recorded so a layout can still be checked once they are deleted ([Oracle_Testimony.md](../guides/Oracle_Testimony.md)). Do not move it or repoint `the_recorded_testimony_holds`. The exception covers the testimony alone. Open: the metadata schema snapshot store that `just snapshot` writes also sits in `openrage-corpus/or-meta/`, and the owner has not ruled on it. Superseded on 2026-09-26 by [OD-TESTIMONY](#the-testimonys-frozen-key-sits-beside-it-and-recording-ends-2026-09-26) and [OD-BASELINES](#r66es-behaviour-baselines-live-in-openrage-corpus-2026-09-26): "the exception covers the testimony alone" no longer holds, because it also covers the testimony's frozen key and R66e's baselines. The open question of the schema store answered on 2026-09-26 by [OD-SCHEMA-LOC](#the-editor-and-the-cli-find-the-schema-snapshot-by-a-fixed-order-2026-09-26): it stays.
- The binary primitives stay: `PgPtr`, `AtArray`, `DatBase`, the `PgBase` family, two dictionary and string headers and the math types, now `or_io::prim`. They are primitives, not covered structs.
- YPDB is undecided: reading it as covered was the design applying his provenance test, and it waits on him. Answered on 2026-09-26 by [OD-YPDB](#the-pose-database-is-ordinary-code-2026-09-26): ordinary code.
- Byte-identity is not the goal for every format, because chasing it produces bad code. Where it is not viable, save and export write OpenRage's own deterministic canonical form, so re-importing and re-exporting an unchanged file is byte-identical from then on.

### The pack is read everywhere, names are a last resort, and a save is a canonical fixed point (2026-09-24)

The owner's answers to the scope questions the 2026-09-23 ruling raised. Rulings 1 and 4 superseded on 2026-09-24 by [D1](OR_IO_Specification.md#owner-decisions): GTA V's structures are typed Rust in `or-io`, whose declared names are the vocabulary, so roles were withdrawn (R65u). The machinery the rulings produced is or-io's generic path ([what carries over](OR_IO_Specification.md#what-carries-over-from-the-pack-native-design)).

1. Pack reads everywhere, as fast as reading the pack only allows: bind once, read `base + offset`, and keep no consumer-side struct unless a measurement shows no pack-native path meets the need. None was claimed: the spike read a field through a pack-built type plan in 0.155–0.157 ns against a typed struct's 0.077–0.080 (release build, corpus `PtxEffectRule`, at `844d3c7c`).
2. The binary primitives stay (corrected above).
3. Scope is provenance: every RSC7 resource format, the RSC7 header, RPF7 and GXT2 are covered; DDS, FXC and AWC stay ordinary code; YPDB is undecided (answered on 2026-09-26 by [OD-YPDB](#the-pose-database-is-ordinary-code-2026-09-26)). Its predicted consequence, a no-pack build that opens no archives, went with the pack-only route: RPF7 is read by `or_io::container::rpf` without a pack.
4. A name in application code is a last resort; the design proposed roles, an OpenRage-owned vocabulary bound by the pack.
5. `openrage-corpus` holds game files, with the testimony exception above; synthetic test packs with invented types are allowed.
6. Byte-identity with the original is not a goal: the first save may move bytes, and saving an unchanged document again is byte-identical and deterministic.

### OpenRage holds no RAGE resource structs, and `.ypt` and `.ycd` are not exceptions (2026-09-23)

Superseded on 2026-09-24 by [D1](OR_IO_Specification.md#owner-decisions).

The owner reversed V2's key decision, that the structs stay and only the offsets go: `openrage` would hold no RAGE resource structs, compiled or as typed views, and every consumer would work on pack-described trees. The write rows recorded as permanent, `.ypt` (behaviour keyframe tables point inside their own allocation) and `.ycd` (`AtMapEntry::data` serves two element types), were reopened by extending the pack format, and both write through the pack since R65f and R65g. A wrong pack stayed worse than a missing one, and decode rules stayed in code.

### Each document keeps its own camera; a workspace stays a layout preset (2026-09-23)

Decided by the owner for R59e: what a user notices losing when they switch is the camera, not the workspace.

- A workspace is one of the closed `Workspace` variants: a panel arrangement over the one document list, chosen by document kind (`for_kind`, `handles_kind`), whose only state is the dock arrangement `src/ui/dock_layout.rs` persists. It holds no documents, scene or camera. A workspace as a scene container was not taken: it turns the closed enum into a collection users manage and touches the whole document model, for nothing a user asked for.
- Each open `DocumentId` has one `OrbitCamera` in `src/state/viewport_camera.rs`, stepped once a frame by `step_viewport_camera`. A document seen for the first time is framed on its bounds; after that only input, the frame key or choosing another dictionary entry moves it.
- ⚠ Keyed by document, never by workspace: a camera per workspace gives one document several views and still loses them all on a document switch (`a_documents_camera_survives_switching_documents_and_workspaces`).
- Cameras do not persist across a restart: a `DocumentId` names nothing in the next process. Keying them by path in `prefs.json` would make a per-file memory that outlives the file's contents, grows without bound, and sits in the one document where a single unreadable value once cost every preference. A persisted camera belongs in the `.orproj`'s per-document entry, read through a `serde_json::Value` as the dock layout is.

### Hosted CI is Linux per pull request; the Windows suite runs on each tip of `main` (2026-09-23)

Hosted CI is the cheap, fast signal: every push to a non-draft pull request runs fmt, the lints, clippy and the workspace tests on hosted Linux, and the full Windows suite runs on each tip of `main` on the self-hosted runner (`main-tip.yml`) and weekly on a clean hosted machine. The whole workspace on hosted Windows for every push had cost $151.65 from 2026-09-20 to 09-23, $124.47 of it Windows (billing API), and hit the spending limit, which stopped every hosted job for seventeen hours. Do not move the full suite back (#170).

### A plugin is WASM in the app, and a subprocess outside it (2026-09-21)

The plugin surface is two surfaces with different purposes, because one mechanism would have to be either safe or capable; the roadmap files each item under its surface (R43 to R47).

- In the app, WebAssembly: untrusted by default, capability-controlled, many installed at once. A community plugin must not read the user's documents, reach the network or take the editor down, which holds only if the capability set is small and stated (R44). It sits in the UI and cannot touch a GPU or a native library.
- Outside it, a subprocess speaking NDJSON, launched the way Windows Explorer's "open with" works, for tooling that wants a whole process; it can use any language and GPU but not the editor's UI. This half largely exists: every route of `openrage-cli serve` reads except `wardrobe-add`, `wardrobe-set`, `wardrobe-fix` and `wardrobe-waive`, which change the clothing pack of a project the caller names and never the install ([CLI_Server_Protocol.md](../reference/CLI_Server_Protocol.md); `crates/tools/or-cli/tests/the_external_surface_is_what_it_documents.rs` fails when any other route writes), so what is left is a stability commitment and the convert routes (R43).
- The clothing tool is the in-app forcing case: the `archive/ydd-clothing-preview` tag adds inspector panels and LOD statistics beside the viewport's preview, which a separate process could not do without reimplementing the view.
- No Rust type crosses the WASM boundary, so the host API is the pack's vocabulary, a described tree with offsets, widths and enum tables (`or_io::desc`), never a compiled struct.

### R15 is a self-hosted runner on the owner's machine (2026-09-21)

The checks that need the private packs and the corpus run on a self-hosted GitHub Actions runner on the owner's machine, `openrage-local-win`, labelled `openrage-corpus`, as the `layouts` job of `.github/workflows/local-gate.yml`; a hosted runner can hold neither, and nothing is uploaded. It runs the pack conformance suites, the corpus-gated suites, `no_shared_type_diverges_in_offsets_alone` (which needs both packs) and the clean-checkout run.

- ⚠ The pack crate's relative path dependencies resolve beside the pack repository, so its suites run in place check the pack against the sibling checkout, and a pull request that breaks a layout passes. The workflow stages the pack sources beside the pull request's checkout and fails if the path resolves anywhere else.
- A coherent wrong offset on a type both packs describe is caught, and `a_broken_layout_cannot_pass_this_gate` re-applies the perturbation every run. A moved offset between two scalars of a type only one pack describes is caught by nothing; `the_gap_is_named_and_still_open` exercises it.

Its consequence, unblocking further `pack_only` conversion and R10's Gen9 collapse, is superseded on 2026-09-24 by [D1](OR_IO_Specification.md#owner-decisions): new format work goes to `or-io`, `pack_only` goes with `#[rage_struct]` in R66f, and the collapse into the pack (R65o) is closed.

Superseded on 2026-09-30 by [the split](#the-pack-tooling-goes-before-pack-discovery-2026-09-30) and R66f: `layouts` stages no pack and runs no pack suite; it runs `or-io-verify` on the corpus, the behaviour baselines and the clean-checkout run.

### The `pack_only` migration lands, and R15 becomes the item that pays for it (2026-09-21)

Superseded on 2026-09-24 by [D1](OR_IO_Specification.md#owner-decisions): no type is converted further, and `pack_only` goes with `#[rage_struct]` in R66f.

R7's migration made pack binding the only read path for the types it converted, removing the bind-time offset comparison, the per-read differential against the compiled reader, and the compiled reader. The pack repository's conformance suites do not replace the comparison: for a `pack_only` type they compare field sets, tags and variants, not offsets, and `require_coherent` refuses only a field past its type's end or two fields sharing a byte. Swapping `RmcLodGroup`'s `bounds_box` with `lods` in a real Gen9 pack was accepted by `bind`, and the staged Gen9 `.ydr` files still opened; only the differential against the compiled Gen9 reader noticed. Decision: land the migration and promote R15 to Phase 1's blocking item, a bounded exception to "a wrong pack is worse than a missing one" with a named remedy. [Oracle_Testimony.md](../guides/Oracle_Testimony.md#what-checks-a-layout-and-what-does-not) states what the checks cover today.

### Provenance is not a gate (2026-09-20)

This is a research project for the owner's own use for now, and provenance questions do not block work. R3 shrinks to a light provenance record, and R54 is unblocked.

- CodeX is treated as free and open source; its ports need no remediation. Superseded on 2026-09-24 by D7: the ports stay on their authors' permission.
- PyMateria may be installed and used, including for benchmarking; OpenRage is not considered to compete with it.
- The packs repository is private; `.orpack` files are distributed by the community, not by the project. Superseded on 2026-10-01 by [the repository's deletion and the export's removal](#openrage-packs-is-deleted-and-the-orpack-export-goes-2026-10-01).
- Publishing is not a near-term concern. Superseded on 2026-09-24 by D3 and D8: publication is R66h.

The tooling matches: CI, `just lint-provenance` and the pre-commit hook run `scripts/lint_provenance.ps1` without `-Strict`, `just lint-provenance-strict` passes it for anyone who wants citations fatal, and committed game content is fatal everywhere whatever the flag. Narrowed on 2026-09-24 by R66b: a leaked-source citation is fatal inside the paths `scripts/or_io_crates.txt` lists and reported elsewhere. Superseded on 2026-09-30 by R66f (P67): a leaked-source citation is fatal in every tracked file, because the whole repository is published with the crate (D3, D8), and `-Strict`, `just lint-provenance-strict` and the crate list are deleted. On 2026-09-20 the tree cited the leaked engine source by path in 71 tracked files; they are out of the tree, and removing them from the history is R66g.

Superseded in part on 2026-10-01 by [the owner's ruling on R3](#no-provenance-record-is-kept-and-the-provenance-lint-stays-2026-10-01): no provenance record is kept, and the grammar and every `Provenance:` block go.

### Documentation counts become assertions (2026-09-20)

Hand-maintained numbers in documents go stale by default. Derivable counts become generated assertions in the style of `src/shell_budget.rs`, definitional ones are deleted rather than corrected, and corpus-backed ones stay prose that names its suite and corpus revision. [Documentation_Standard.md](../guides/Documentation_Standard.md) applies the rule to every document.

### `feat/v2-overhaul` is the source of truth (2026-09-19)

`main` was six weeks stale and had no descriptor-pack system, so the live tree was `feat/v2-overhaul`. Done on 2026-09-20: it landed on `main` through #1, and `main` is the live tree.

## Working preferences

- Work runs on the owner's machine (Windows), in parallel when it makes sense, with the checkout, the corpus and the packs side by side; orchestration runs there too. Start the worker with `agent worker start`, not `cursor worker start`. Cloud VMs are for parallel work that needs only source and GitHub.
- `openrage-corpus` is an unversioned directory on that machine only, staying so for now; `openrage-packs` was archived and deleted on 2026-10-01 ([R66i](Roadmap.md#r66i-retire-openrage-packs)). Fixture-gated suites and anything touching a real game install run there by necessity.
- Plans and reports are delivered as documents with a short summary in chat, not as long chat messages.
