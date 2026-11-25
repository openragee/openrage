# Workbench views design

What a person would see in the weapon, particle, map and other project views, what each needs underneath, and the order they could be built in: a draft for the owner to approve.

Status: draft of 2026-10-19, wave P32 of [Workbench_Plan.md](Workbench_Plan.md), answered by the owner on 2026-10-20. Of it, wave P40 built the table of task screens ([step 0](#the-order-it-could-be-built-in)), with the Wardrobe as its first row, and wave P42 the Weapons view, which [says what it built and what it left](#what-wave-p42-built-for-weapons) and answers the questions the draft asked of it, and wave P43 the Particles view, which [says what it built](#what-wave-p43-built-for-particles), and wave P44 the Maps view, with the placement of objects with a manipulator that the owner asked for, which [says what it built and what it measured](#what-wave-p44-built-for-maps). No other kind of content or template is built. [Workbench_UX_Design.md](Workbench_UX_Design.md) owns the screens that exist and [Project_Model_Design.md](Project_Model_Design.md) the model of content; this document drafts what they would hold next. [The rulings it asks](#rulings-the-draft-asks-of-the-owner) are at the end.

## Verdict

A view is a task screen over a kind of content, and the three views the owner asked for first need three different amounts of new screen, so the draft sizes each one before it draws it. The order of the views is the owner's, weapons, particles and maps and then the others (requirements PRO-180 and WAR-220 to WAR-223 of [the open list](../audit/workbench-open-2026-10-20.json)).

- **Weapons** is a task screen shaped like the Wardrobe: weapons and their components in a list, the weapon assembled in the middle, its metadata on the right. It was the most work, because the tree declared no typed weapon metadata and a screen can only be as good as the names under it: wave P42 built it, family first ([what it built](#what-wave-p42-built-for-weapons)).
- **Particles** is the Particle workspace laid out for a project. The editor, the simulation and the timeline are built, and every panel is an item of the registry that any layout can dock, so a second workspace would add a button and a layout and give a `.ypt` two homes. The draft recommended one panel, the effects of every file in the project, a template and a layout for a project, with no kind of content of its own; the owner changed that on 2026-10-20 (V4) to a source form and a kind of content as well, and wave P43 built all five ([what it built](#what-wave-p43-built-for-particles)).
- **Maps** has the most data under it: the world model, its checks, the manifest derivation, the metadata sources and create and delete are built, and the app shows a map only through the generic metadata editor. Its screen is a plan of what the project places, the rules the world model already holds, and the packaging. The draft left placing with a gizmo to a plugin ([R47](Roadmap.md#r47-the-plugin-slate-after-the-first)); the owner changed that on 2026-10-20 (V5), and wave P44 built the Map screen with a manipulator in the app's own viewport ([what it built](#what-wave-p44-built-for-maps)), leaving the plugin route available and no longer the only one.
- **The others** are ranked by a stated criterion: vehicles, then props and interiors, audio, animations, graphics packs and scripts. Props and interiors ride on the Map kind, and scripts are a template and not a view.
- **One change came first, and is built.** The app matched on the kind of content in the places [listed below](#what-a-view-costs-in-the-tree), so a second task screen would have copied the Wardrobe's special cases. The table of task screens, one row each (ruling V2), holds them now (wave P40).

## What this draft rests on

The rulings of [Workbench_Plan.md](Workbench_Plan.md#rulings-in-force) hold and are not reopened: a project has no type, content and builds are the screen words, a project holds sources and not compiled game files, and a launch reopens the last project. Held back stays held back: importing CodeWalker XML for drawables, author mode, the shader and vertex-layout presets that ship, and any claim that a build loads in game. A build reports that every file it wrote reloads through the readers, and no view in this draft says more.

The code is the truth, and every claim about the tree cites a path or a test. Where the tree states no measurement, the draft says so and does not supply one. It names no feature of a competing tool that the repository did not record: the repository's own record of each tool is in the bar each view sets.

## What a view is made of

### The parts of a view

A view is five things, and the Wardrobe is the one example that exists.

| Part | What it is | Where the Wardrobe has it |
|---|---|---|
| Kind of content | A `ModuleKind`, its folder and manifest, its rules and what it contributes to a plan | `crates/domain/or-wardrobe` and `crates/domain/or-addonped` |
| Template | A starting set of content and builds, the screen a project opens on and the buttons it pins | `Template::Clothing` in `src/state/front_door/new_project.rs` |
| Workspace | A layout of panels. A task screen draws no document, and it holds no scene: the dressed ped is a panel | `Workspace::Wardrobe` in `src/ui/dock.rs`, `src/ui/panels/wardrobe/` |
| Empty state | What the screen lacks and the action that gives it something | `heading` in `src/ui/panels/empty_state.rs` |
| Builds | The kinds of build that can ship the content, with a locked cell and a sentence where one cannot | `cannot_ship` in `crates/domain/or-project/src/plan.rs` |

⚠ A workspace stays a layout and not a scene container (the owner's ruling of 2026-09-23, [Project_Context.md](Project_Context.md)). An assembled weapon, a plan of a map or a vehicle with its liveries is therefore a panel that a workspace docks, and each document keeps its own camera.

### What a view costs in the tree

The Wardrobe was built when it was the only task screen, and its kind was matched by name where the app decides what to show. Wave P40 moved every one of those places onto the table of task screens (`src/ui/task_screens.rs`, `TaskScreen`), so a second view adds a row beside the Wardrobe's and none of them changes. Wave P42 added the second, Weapons, with no line in any of them (`no_site_matches_on_the_kind_of_a_task_screen` holds the places to it), and [the recipe for a kind](../reference/Crate_Reference.md#what-a-kind-of-content-costs) is written from it.

| Where | What it decides | How it reads the row | Path |
|---|---|---|---|
| The workspace | Whether it is a task screen, whether it needs a project, its layout and the panels a saved layout must keep | `is_task_screen`, `needs_a_project`, `layout` and `must_dock` | `Workspace` and `layout_for` in `src/ui/dock.rs`, `restore` in `src/ui/dock_layout.rs` |
| What is offered | A task screen is offered only while the project holds its kind of content, and the window leaves it when the last goes | `workspaces_offered` and `follow_the_offered_workspaces` ask the row for the kind | `src/state/workbench/strip.rs` |
| A click on a Content row | Opens the content's screen on that content | `show` and `workspace` | `content_row` in `src/ui/panels/project/content.rs` |
| Open in | Says whether each workspace can show the content or a row of the Assets table, and why not | `kind`, `other_content` and `its_rows` | `where_content_opens` in `src/ui/panels/project/content.rs`, `where_it_opens` in `src/ui/panels/project/mod.rs` |
| The empty state | The heading and the command that adds the content | `empty`, `add` and `add_button` | `heading` in `src/ui/panels/empty_state.rs` |
| Words and counts | The name of the content, the word its count is said in and how it is counted | `content`, `count` and `held` | `src/state/workbench/tally.rs` |
| The start page | The badge of a card, and the chip of the template | `content`, `count`, `on_disk` and `starts` | `src/state/front_door/cards.rs`, `src/ui/panels/start_page.rs` |
| The template | The screen it opens on and the buttons it pins | `starts` | `Template` in `src/state/front_door/new_project.rs` |
| Ways in | The Project menu's submenu, the line under the add command of an empty project, and what a drop says | `menu`, `landing_note`, `drop_hint` and `drop` | `src/ui/panels/menu.rs`, `src/ui/panels/project/assets.rs`, `src/ui/panels/status_bar.rs` |
| Where a data source ships | The folder of a resource at which a text `.meta` of the content is registered (ruling V3) | `data_sources`, none for a content that keeps no such source | `src/ui/task_screens.rs` |

What a second view still adds is the part that is its own, and the wave that builds it adds nothing to the places above:

- **The name.** A variant of `Workspace` with its label and tooltip, listed in `Workspace::ALL`; the workspace is stored by that name, so a variant adds a name and removes none, as `Wardrobe` did.
- **The kind.** A `ModuleKind` variant, its `Content` impl and its line in `kinds` of `crates/domain/or-content/src/lib.rs`, and a `Template` variant with its words when a template starts it.
- **The row and its panels.** The row, its layout function and the panels the layout docks.
- **What reads the content with its engine.** The files that read a clothing pack with `or_wardrobe` (the scan, the check of a built pack, the sizes of a build, the matrix of locked cells, the options of a build, and the question a drop of clothes asks) name the kind and are listed with their reasons in `no_site_matches_on_the_kind_of_a_task_screen`. A second kind's engine is read the same way, or through `Content` when its plan is the generic one.

`a_second_task_screen_is_one_row` is the proof: it adds a stand-in row for one test and shows it in the strip, the palette, Content, Open in, the start page, the empty state, the drop hint, the layout and the Project menu with no line of its own in any of them. A kind the build does not know is carried and refused by name, not dropped (`a_module_the_build_has_no_kind_for_is_refused_by_name_and_not_dropped`, which uses `weapons` as its example), so a project file written by a later build is safe in this one.

The strip is the bound. Ctrl+1 to Ctrl+9 reach nine workspaces, the palette and View ▸ Workspace reach the rest, and a project that held clothing, weapons and a map would offer ten. That is one reason the Particles view adds a panel and not a workspace.

⚠ The Project menu lists one submenu for each row, and a popup does not scroll (`project_menu_of_the_window` keeps its rows few so that every one is on screen in the smallest window), so the commands of a screen stay few.

### How a view is reached

Every door that exists for the Wardrobe is the door of the next view, and none is new.

- **New project…** (File, the start page and the palette; it has no default key, and Ctrl+N is unbound, as `new_session_and_maximize_are_listed_and_unbound` in `src/ui/shortcuts.rs` holds) offers a template, and a project of one kind of content opens on that kind's screen while one of several opens on Project (ruling R8, built as `workspace_for` in `src/state/front_door/new_project.rs`). The draft keeps it and asks the owner to confirm it as a ruling.
- **A click on a Content row** suggests the content's screen, and **Open in** lists every workspace, each saying under its name why it cannot show the content and opening anyway (`where_content_opens`).
- **A double click in the Assets table** opens a file in the workspace of its kind: a weapon's model opens in Model, a `.ypt` in Particle, a `.ymap` in Data. From there Open in offers the view.
- **The strip** offers a task screen only while the project holds its content, and the palette and View ▸ Workspace reach every workspace.
- **A drop** adds to the content of the screen under the pointer, and the hint under it says so, as the Wardrobe's does (*Add as clothes*); a drop elsewhere only opens.
- **The start page** has a chip for each template a row declares ([what the front door builds](Workbench_UX_Design.md#what-the-front-door-builds)): the Weapons content brought Weapon pack with its row, the Particles content Particle pack, and the Map content Map pack.

### What a kind of content hands to a build

A kind plugs into the project through the `Content` trait in `crates/domain/or-project/src/content.rs`, which is a recogniser (it says whether a folder is its own), a `contribution` to a plan and a `start` for an empty folder, and through one line in `kinds` of `crates/domain/or-content/src/lib.rs`. A `Contribution` (`crates/domain/or-project/src/contribution.rs`) is data: files and documents at their paths in the resource, manifest lines, names the game resolves once (claims), named figures, items with a stable id, problems and notes.

The crate of a small kind has a fixed shape, which `crates/domain/or-addonped/src` shows: a model, facts read off the disk, rules as data with a code, a severity and a waiver (`crates/domain/or-addonped/src/rules.rs`), a plan, a verifier that reads a built folder back, a recogniser and an `openrage-cli` verb. A view's kind follows that shape, and its screen is thin: every rule and every write is the crate's.

A build reloads what it wrote through `reload` in `crates/domain/or-project/src/readers.rs`, which reads `.dds`, `.ymt` and every declared RSC7 format and says that a name nothing reads has no reader to try. It does not name `.ymap`, `.ytyp`, `.ymf` or a text `.meta`, so every view whose files are metadata starts with that one small change, shared by weapons, maps and vehicles.

### Sources and the form of metadata

A source folder exists for textures, drawables, drawable dictionaries, fragments and metadata (`.ymt`, `.ymap`, `.ytyp`), and a compiled file with no source form is kept whole under `files/` and joins the Files content ([Source_Format_Design.md](Source_Format_Design.md#layout-per-format)). Bounds, cloth, clips, particles and audio are planned and not built.

Question Q2 of [Wardrobe_Parity.md](Wardrobe_Parity.md#questions-for-the-owner) asks whether a metadata source is the folder that is built or the flat file the first design said. The draft recommends the folder, for the reason the source design gives: a bare `name.ymap.xml` has no `asset.json`, so no stable id to follow a rename, no origin hash to tell the same file twice and no `from_game` for the rule that a build leaves out an unchanged original. Every view below needs those three.

The folder form has one gap that weapons and vehicles meet. A text `.meta`, which is what most weapon and vehicle data is, is kept under `files/` at its own folder path and is not made a source (`a_data_meta_is_kept_under_files_at_its_own_folder_path_not_made_a_stream_source`), because a cooked source ships at `stream/` and a data file is registered by its own path. [The source design](Source_Format_Design.md#metadata) says a row would restore the import once the owner rules where such a source ships. The draft recommends that a data source ships at the path its content's table states, so a Weapons content can hold its `.meta` as a source and a build registers it by that path (ruling V3).

⚠ A source's XML spells every name the public table of `or_io::meta` lacks as `hash_XXXXXXXX`, and a snapshot of the schema is never committed (`crates/io/or-io/src/meta/README.md`). The world family is named in that table (`crates/io/or-io/src/meta/binary/public_names.rs`) and weapons are not, so a weapon source would be unreadable to a person until the weapon family was declared. It is not: the weapon files of both installs are XML text, a source of one is the file's own XML read by the engine's names whatever the host has installed, and wave P42 declared the family for the typed reads and edits.

## Weapons

The Weapons view serves a person who makes a weapon pack: a weapon's models and textures, its metadata, and the parts that attach to it.

**What the person sees.** A task screen with the Wardrobe's shape and a different middle.

```text
+- Weapons - my_arsenal ------ build: FiveM resource [Legacy] ----------- [Problems 2] [Build] -+
| Project          | Weapons                | Assembled weapon                | Weapon           |
|  Content         |  [Add weapons] [group] |                                 |  name   id kept  |
|   Weapons  3     |  v Pistols             |   +-------------------------+   |  model  textures |
|   Files    2     |    > <weapon>     ok   |   |                         |   |  [Info][Parts]   |
|  Builds          |        <component>     |   |  the weapon, with the   |   |   fields in the  |
|   * my_arsenal   |        <component>     |   |  chosen component worn  |   |   engine's own   |
|                  |    > <weapon>      !   |   |  at its attachment      |   |   groups         |
|  Outline         |  v Rifles              |   |  point                  |   |                  |
|                  |    > <weapon>     ok   |   +-------------------------+   |  Components      |
|                  |                        |  Part [none][<c1>][<c2>]        |   <c> -> <bone>  |
|                  | drop: Add as weapons   |  Skeleton [x]  Before/After     |  Effects, sounds |
+--------------------------------------------------------------------------------------------+
| Problems | Build | History | Jobs | Log   <weapon> held twice, <texture> missing, <bone> absent |
+--------------------------------------------------------------------------------------------+
```

The header holds Add weapons, a filter by group and by name, the build chooser and Build. The list is grouped by the group the metadata states, with the components of a weapon under it and a chip for what needs attention. The middle is the weapon assembled from the pack: orbit and zoom, a part chosen from the weapon's components and worn at its attachment point, the skeleton overlay, and Before and After as the Wardrobe's preview has them. The right panel has the weapon's name and stable id, its model and textures, the fields of its metadata, laid out by the panel and shown by the Visual editor where it has laid none out, and its components, each with the bone it attaches to. Effects, sounds and animations a weapon names are shown as references with an Open that goes to the Particle, Data or Animation workspace; they are not content of the Weapons kind. Along the bottom are the tabs of the Project and Wardrobe layouts.

**Content kind.** `ModuleKind::Weapons`, written `weapons` in the project file, with a folder of the project and a manifest in it, `weapons.json` in this sketch (names are illustrative, as [the on-disk sketch](Project_Model_Design.md#on-disk) of the model design is). The unit is a weapon with a stable id, the name the game resolves it by, the files of its model and textures, the metadata sources that define it, and its components, each with the attachment point it uses. The study of clothing names the unit of weapons as a component and an attachment point ([Clothing_Tool_Design.md](Clothing_Tool_Design.md#the-project-system)), and the schema parser records the engine's own constant for the number of attachment points a weapon info holds (`crates/io/or-io/src/meta/schema/types.rs`).

What the kind contributes to a plan: the model and texture files at `stream/`, the metadata files with the `data_file` lines that register them, each weapon's name as a claim of its own kind, figures (weapons, components, bytes of textures, estimated memory) and an item per weapon and component, so that a rename is a move in the build's diff.

Typed metadata is the first piece of work and the one that decides the rest. The tree declares the ped variation family and the world family by hand, found by the name hash each document's own type table stores (`crates/io/or-io/src/meta/typed/ped.rs`, `crates/io/or-io/src/meta/typed.rs`), and no weapon family: `crates/io/or-io/src/meta/binary/root_types.rs` lists `CVfxWeaponInfoMgr`, the root of the effects a weapon names, and no root for a weapon's info. Weapon metadata opens today in the Visual panel of the Data workspace, with the names a local snapshot gives it and `hash_XXXXXXXX` without one. The family is declared as the ped family was: names from the engine through the corpus, the testimony and the local layout references (D15 of [OR_IO_Specification.md](OR_IO_Specification.md)), a writer that grows an array, and a fixed point checked over every weapon file of the installs. ⚠ The staged metadata corpus holds no file of a weapon's info (its listing of 2026-10-19 has `vfxweaponinfo`, `weaponevents` and `weapontasks` among its `.ymt` files), so the measurement is made on a mounted install, as the world family's was ([R32](Roadmap.md#r32-world-data-ymap-ytyp-and-interiors)), and the wave says which suites that makes install-gated.

Rules, as data in the shape of `crates/domain/or-addonped/src/rules.rs`. The ones this draft proposes are the ones an existing check already supports, and each is added only with its measurement.

| A rule would say | The check it rests on |
|---|---|
| A weapon's name is held twice in the project | The claim machinery of the plan, which refuses a name two contents claim |
| A weapon names a model or a texture that neither the project nor the game has | The dependency index behind the Uses panel (`src/ui/panels/hierarchy/uses.rs`, `crates/domain/or-depends`) |
| A component attaches to a bone the weapon's skeleton lacks | The skeleton reader (`crates/io/or-io/src/gta5/skeleton.rs`), once the declaration says which field names the bone |
| A metadata file does not read back | The build's verification, after the reload hook above |
| A weapon's textures take more memory than a limit the project sets | The `memory` figure of `Verification` in `crates/domain/or-project/src/build.rs` |

**Template.** *Weapon pack*, already named in the template gallery of [the UX design](Workbench_UX_Design.md#create-a-project): content Weapons with an empty folder, one FiveM resource named after the project, opens on Weapons, and pins Project, Model, Texture and Weapons as the Clothing pack pins its four.

**Panels it reuses, and what it adds.**

| Reused as they are | Added |
|---|---|
| Project, Outline, Problems, Build, History, Jobs and Log, in the layout of `wardrobe_layout` | A list of weapons and components (`src/ui/panels/wardrobe/list.rs` is the precedent) |
| The viewport, the skeleton overlay and the bone editing of the Model workspace | An assembled-weapon panel, over a pure query from the pack and a choice to the models and textures worn, tested without a renderer as `crates/domain/or-wardrobe/src/dress.rs` is |
| The Visual metadata editor, for any field the Weapon panel has not laid out, and presets, which apply one weapon's chosen fields to a selection in one step (`src/state/presets.rs`) | A Weapon panel, over the typed family |
| Texture replacement from an image, with its rules, and Replace geometry… for a model | Rules, a verifier and the `openrage-cli` verbs, as a kind's crate has them |

**Builds.** A FiveM resource ships it with no further question. A singleplayer DLC ships it when every metadata file states the type it registers as and every model sits in an archive folder, which is the rule `refuses` in `crates/domain/or-project/src/dlc.rs` and the Builds window already shows as a locked cell with its reason; `WEAPONINFO_FILE` is the type a retail pack registers a `.meta` as most often, though no spelling is a majority ([Dlc_Pack_Creation.md](../features/Dlc_Pack_Creation.md#traps)), so the kind states each type from a measurement and invents none. An `.oiv` ships what the plan lists. Whether a pack's weapons load in game is the line no build ticks.

**Empty state and ways in.** The heading reads *No weapons in this project* and the actions are *Add a weapon pack to this project*, *Add weapons…* (models, textures and `.meta` files, each game file decompiled at once) and *Open a weapon file…*, with the line that files dropped here are added as weapons. The ways in are the doors above.

**Edited here, in a plugin, elsewhere.** In OpenRage: the metadata fields, the component table, names and ids, textures by replacement, a model's geometry from glTF, GLB or FBX in edit mode, a bone's numbers, and the effects a weapon names through the Particle workspace. In a plugin, if anyone writes one: balancing helpers and framework configs, which are data in and out ([Plugin_ABI.md](../reference/Plugin_ABI.md)). In another tool: the modelling and rigging of the weapon and its parts, the audio, and the test of how it feels in game.

**The bar.** The repository names CodeWalker, CodeX and OpenIV as the tools OpenRage replaces ([Roadmap.md](Roadmap.md)) and records weapon assembly as one row of the union list ([R47](Roadmap.md#r47-the-plugin-slate-after-the-first)); it records nothing of how any of them edits a weapon, and this draft claims nothing of it. The bar is five checks OpenRage can fail: a weapon file opened and saved with no change saves in the canonical form and saving that again is byte-identical; every field shows under the engine's name on a clone with no snapshot; the build reloads every file it wrote; a name held twice or a missing model is found before a build; and `openrage-cli` says the same headless. The first is the repository's rule for every save, and the other four are what a project can check that one file cannot.

**Not in the tree yet.** What [the subsection below](#what-wave-p42-built-for-weapons) lists as left: the skeleton overlay and Before and After, an Open a weapon file… door, and the rules that need a model's bones or the memory of a texture.

### What wave P42 built for Weapons

The three steps W1, W2 and W3 of [the order below](#the-order-it-could-be-built-in), each ended on a check seen failing first. The screen is described under [what the Weapons workspace builds](Workbench_UX_Design.md#what-the-weapons-workspace-builds), and the recipe the next views follow is [what a kind of content costs](../reference/Crate_Reference.md#what-a-kind-of-content-costs).

- **The family (W1).** `or_io::meta::typed::weapon` declares the three roots, `CWeaponInfoBlob`, `CWeaponComponentInfoBlob` and `CWeaponModelInfo__InitDataList`, with the ammunition, component, firing, aiming and camera records under them, as owned records in the PSC's dialect (`typed::weapon_types`), so a weapon opens where no schema snapshot is installed. ⚠ The sketch assumed the files were binary like the ped family's, and they are not: every weapon file the suite opened is XML text, so the family reads and writes text, a record keeps the value tree it was read from, and what no member names arrives in a save as it came: a `ref` to a named item, a null pointer, a member whose shape differs from file to file, and a value that does not read as its type (the install has one vector written `0.2.000000`). A save is OpenRage's canonical XML, which the game reads as it reads the shipped text. The animations, loadouts, pickups and shops that the same folders hold are other roots and are references, not typed.
- **What was measured, and where.** On the owner's Legacy install, 311 files of the three roots (220, 44 and 47) and on the Enhanced install 317 (223, 46 and 48) open, save, lose no element the file stated (the two texts compared as lists of elements, numbers by value) and save that text to the same bytes, and every member the family declares is one the install states (`every_weapon_file_of_the_install_opens_saves_and_arrives_as_it_was`, install-gated: it reads `OPENRAGE_GAME_ROOT` and the skip ledger names it when none is set, because the staged corpus holds no weapon file). The type each root is registered under is read off the install's own registrations (`a_weapon_file_registers_under_the_type_its_root_names`, install-gated). With no install the suites in `crates/io/or-io-verify/tests/meta/typed_weapons.rs` hold the family to documents they write: names read, an untouched save a fixed point, an edit changing one value, a list growing by a record built here and a weapon built from nothing reading back (`a_list_grows_by_a_record_built_here_and_a_weapon_built_from_nothing_saves_and_reads_back`), and a snapshot changing nothing (`a_staged_schema_snapshot_changes_nothing_a_weapon_file_reads_or_saves`). The names are the engine's, in Rust casing (D15). Not measured: a file of the family that the install does not ship, and what the game does with a value.
- **The content (W2).** `or-weapons` is the crate of [the shape above](#what-a-kind-of-content-hands-to-a-build) and `ModuleKind::Weapons` its kind, `weapons` in the project file, with `weapons.json` in its folder. `or_content::kinds` lists it, so the editor and `openrage-cli` plan it the same way. The rules are data (`or_weapons::RULES`): a weapon with no name or a name held twice, a source that escapes the folder, is missing or cannot be cooked, metadata that does not read or does not define the weapon, two sources cooking to one file, no metadata, no model, a model other than the one the metadata gives, a source no weapon names, a component no metadata defines (a note), and a waiver with no reason or none that matches. Of the five rules the sketch proposed, two are built as proposed (a name held twice, and a metadata file that does not read back, which the reload hook and the build's verification add to), one in part (a model or a source a weapon names is found missing against the project's own sources, and not against the dependency index of the install) and two are not: a component attached to a bone the skeleton lacks, since no reader of a model source gives its bones, and a limit on the memory of a weapon's textures.
- **Where a data source ships (V3).** At `data/`, `DATA_FOLDER` of `crates/domain/or-weapons/src/layout.rs`: a text `.meta` of the content is a source (`<name>.metadata.src`, made by `or_weapons::import`, since the general import keeps a `.meta` under `files/`), a build ships its cooked text there and registers it by that path under the type its root names (`WEAPONINFO_FILE`, `WEAPONCOMPONENTSINFO_FILE` or `WEAPON_METADATA_FILE`), and a model or a texture ships at `stream/`. The folder is the row's choice and not a measurement, since FiveM reads a data file by the path its manifest names; what the tree measures is the type each root registers as. The row of the table states it (`data_sources` in `src/ui/task_screens.rs`).
- **Build cells.** Derived, as ruled (V6): a FiveM resource ships it, and a singleplayer DLC is locked by `cannot_ship` for as long as the plan holds a file with no type, which a model at `stream/` is, and says which file; a pack of data files that all state their type is not locked (`a_singleplayer_dlc_cell_is_derived_from_what_the_content_hands_over`). The design's sentence, that a DLC ships a pack whose models sit in an archive folder, stays unbuilt. An `.oiv` ships what the plan lists. No build says a weapon loads in game, and the verifier says only that each source reached its place, that each data file reloads as its root and defines the weapons, and that the manifest registers it.
- **Headless.** `openrage-cli weapons validate` and `verify` (`a_weapon_pack_is_validated_and_a_word_that_is_no_command_is_a_usage_error`, `a_build_of_weapons_is_read_back_against_the_weapons_and_a_tampered_one_fails`); the plan and the build are `openrage-cli project`.
- **The screen (W3).** One row of the table of task screens, a template, a chip of the start page, six commands, three panels and one worker for the scene, with its empty state and French. The install's own `WEAPON_PISTOL`, with its model and textures, imports into a pack and its scene reads (`a_weapon_of_the_install_is_imported_and_its_assembled_view_is_read`, install-gated); a screenshot of it, drawn, was looked at and is not kept in the repository. Of the sketch, the skeleton overlay, Before and After, and the Open a weapon file… door are not built, and the filter is by name and not by group.
- **Left alone, and reported.** The typed read of a weapon file that is binary (none was found in either install), a weapon's effects as content of a kind (they stay references with an Open, as V7 ruled), and any claim that a weapon works in game.


## Particles

The Particles view serves a person who makes effects: a `.ypt` holds effect rules that fire emitters, emitter rules, particle rules and the textures and drawables they use (`crates/io/or-io/src/gta5/particle.rs`).

**What the person sees.** The Particle workspace as it is, with the project around it.

```text
+- Particle - fx_pack ----------------------------------------------- [Problems 1] [Build] -+
| Project      | Effects                | Viewport: the effect playing  | Inspector          |
|  Content     |  <file>.ypt            |   pins, wind, time of day,    |  effect            |
|   Files   2  |   v <effect>      ok   |   weather                     |  emitter           |
|  Builds      |       <emitter>        |                               |  rule, keyframes   |
|   * fx_pack  |       <emitter>        +-------------------------------+  domains           |
|              |   > <effect>       !   | Timeline: events, curves,     |  [Textures]        |
|  Used by     |  <file>.ypt            | rails for drivers and pins    |                    |
|   (later)    |   > <effect>      ok   | Problems | Build | History    |                    |
+-------------------------------------------------------------------------------------------+
```

The left column is the Project panel with an Effects panel beside it: every `.ypt` of the project, the effects each holds and their emitters, with a mark on what needs attention. Choosing an effect plays it in the viewport, a Ctrl-click pins it so several play together (as [Particle_Preview.md](../features/Particle_Preview.md#the-editor-around-it) describes), and the timeline, the Inspector and the texture preview are the ones that exist. Problems, Build and History join the timeline as tabs. A Used by section lists the weapons and vehicles whose effect metadata names an effect, and arrives only after the weapon family is declared.

The preview draws through the pack of the game's shaders, which the app bakes once a game is set. With none, or with a technique the pack lacks, the batch is skipped and nothing else draws the effect, so a view of effects says that in its viewport and not in a log. R58f, which the brief for this wave names as the particle pack work, is that shader baking; the particle code is `crates/domain/or-particle`, `crates/io/or-io/src/gta5/particle.rs` and `crates/runtime/or-simulation`.

**Content kind.** The draft recommended none at first (ruling V4), and the owner changed that: the kind is `particles`, built by wave P43 ([below](#what-wave-p43-built-for-particles)). The draft's reasoning stays as the history of the question. A `.ypt` that a person adds was a compiled file with no source form, kept whole under `files/` and joined to the Files content (ruling R3 of [the second plan](../audit/workbench-waves-2026-10-08.json)), and the build already planned it, copied it and reloaded it through its reader, since `ypt` is a declared format (`crates/io/or-io/src/gta5/formats.rs`). What a content kind of its own gives a project is a rule or a cross-reference that a file cannot give, and its rules start from what `crates/domain/or-particle/src/list.rs` already guarantees, that each rule is held once before a save. The references from a weapon, a vehicle or a ped to an effect need those families declared first, and stay unbuilt.

The source form was the second question, and the one the draft said had to be measured first: [the source design](Source_Format_Design.md#layout-per-format) planned a `.particles.src` of JSON rules and DDS, and the check was whether the rules of a particle list reach a fixed point through JSON. Wave P43 measured it on both installs and built it ([what it built](Source_Format_Design.md#what-wave-p43-built)).

**Template.** *Particle pack*: as drafted, one FiveM resource, opens on Particle and pins Project, Particle and Texture, over the Particles content instead of the Files content, with the layout of the row. It is the first template whose screen is also the editor of a kind of file, which is the `edits_files` field of the row and an addition beside `Template::pinned` and not a change to it.

⚠ Dock layouts are saved for each workspace and not for each project (`SavedLayouts` in `src/ui/dock_layout.rs`, a map from `Workspace` to its arrangement), and the Particle workspace is used with no project open. Docking the project's panels into it would change the window of a person with no project. The recommended answer is a second saved arrangement for the Particle workspace while a project is open, one more map in `SavedLayouts` that older preferences simply lack; the fallback is to dock the panels always and let each say *No project open*. Built as recommended (wave P43): `SavedLayouts::for_a_project` is that second map.

**Panels it reuses, and what it adds.**

| Reused as they are | Added |
|---|---|
| Outline, Inspector, the viewport with its simulation, the timeline and the texture preview of the Particle workspace | The Effects panel: the effects of every `.ypt` of the project, one row per effect, with the file it is in |
| `src/state/sim.rs`, which keeps the manager, pins and drivers, and `src/state/file_watcher.rs`, which reloads an open `.ypt` as it is saved | A layout the template docks, and the Particle pack template |
| The name index that learns rule names from open `.ypt` files (`src/state/name_index.rs`) | An empty state for the workspace when a project holds no effects |

**Builds.** A FiveM resource ships a `.ypt` at `stream/`. A singleplayer DLC and an `.oiv` need a place for the file inside an archive and a type; the cell reads *needs a type* by the rule `refuses` in `crates/domain/or-project/src/dlc.rs`, and what retail packs do with a `.ypt` is not measured in this tree, so the cells stay locked until a wave measures it. Built (wave P43) as drafted, derived by `cannot_ship` and not declared.

**Empty state and ways in.** The workspace with a project and no effect reads *No particle effects in this project*, with *Add particle effects…* (the files picked, each made a source), *New particle effects* (File ▸ New, the row the menu already names *Particle effects (.ypt)*), *Import game files…* and *Open a .ypt…*. Built (wave P43): the screen offers Add particle effects… and New particle effects, and Import game files… makes the same sources. A drop on the Effects panel adds the files to the project, with the hint saying so, and a drop elsewhere on the workspace only opens, as ruling R4 of the second plan has it.

**Edited here, in a plugin, elsewhere.** In OpenRage: every field of an effect, an emitter, a rule, a domain and a keyframe (`crates/domain/or-particle`, saved and reopened by `a_particle_list_edit_saves_and_reopens`), the textures by replacement, and the rules of a list by name. In another tool: the pictures, the models a particle draws, and tuning against the running game. The preview says what it does not draw: lights, fog volumes and decals are not drawn, the mirror, refraction and shadow passes are not, and only a Legacy `.ypt` is simulated ([Particle_Preview.md](../features/Particle_Preview.md#known-gaps)).

**The bar.** The repository names CodeWalker as a tool to replace and measured one thing about it for particles: `tools/cw-compare` loads an original and a re-encoded `.ypt` in CodeWalker and diffs them, to answer whether CodeWalker loads what OpenRage wrote. It is a diagnostic and not a gate, and CodeWalker misreads some formats. So the bar is that a `.ypt` OpenRage saves still loads there, that the simulation follows the engine's solver at full fidelity (the owner's decision D9, recorded in Particle_Preview.md), and, as a check of the project and not of the editor, that the same file is planned, built and read back with its textures counted against a memory figure.

**Not in the tree yet.** Of the sketch, the Used by section, a pin by Ctrl-click in the Effects panel, and the measurements of the places a `.ypt` has in a retail pack ([what wave P43 left](#what-wave-p43-built-for-particles)).

### What wave P43 built for Particles

Steps P1 and P2 of [the order below](#the-order-it-could-be-built-in), and with them the owner's change to V4: a source form, a kind of content and the screen. The screen is described under [what the Particle screen builds](Workbench_UX_Design.md#what-the-particle-screen-builds), and the recipe was [what a kind of content costs](../reference/Crate_Reference.md#what-a-kind-of-content-costs).

- **The source form (P2).** `or_source::particles`, described under [what wave P43 built](Source_Format_Design.md#what-wave-p43-built): one `particles.json`, the textures as the source of a `.ytd` and the models kept whole. The check the draft named first passed on both installs, 1246 Legacy and 1577 Enhanced `.ypt` files each becoming a source that compiles and imports again to the same bytes, with none refused. The game door and the Add door make the same source, and a file the source form cannot hold is refused with the reason and not kept whole instead.
- **The content.** `or-particles` is the crate of [the shape above](#what-a-kind-of-content-hands-to-a-build) and `ModuleKind::Particles` its kind, `particles` in the project file, with `effects.json` in its folder: one asset for each source, with a waiver list. `or_content::kinds` lists it, so the editor and `openrage-cli` plan it alike. The rules are data (`or_particles::RULES`, thirteen): a source that leaves the folder, is missing or cannot be cooked, rules that do not read, two sources cooking to one file, a source named twice, an effect two sources state, an event that names an emitter or a particle rule its list lacks, a texture a particle rule draws that its list does not hold, a source no asset names, no assets, and a waiver with no reason or none that matches. Two sources stating one effect is a warning and not an error, because how the game picks between two files is not measured.
- **Where a source ships.** A cooked `.ypt` ships at `stream/`, as the draft said. A FiveM resource and an `.oiv` ship it; a singleplayer DLC is locked by `cannot_ship` for as long as the plan holds a file with no type, which a `.ypt` is, and says which file (`a_singleplayer_dlc_cell_is_derived_from_what_the_content_hands_over`).
- **Headless.** `openrage-cli particles list`, `validate` and `verify` (`a_particle_pack_is_listed_and_validated_and_a_word_that_is_no_command_is_a_usage_error`, `a_build_of_particles_is_read_back_against_the_files_and_a_missing_one_fails`); the plan and the build are `openrage-cli project`.
- **The screen (P1).** One row of the table of task screens over the Particle workspace that already existed, which is the first row with `edits_files`: the workspace stays the editor of any `.ypt`, with its own name, no project needed and its own saved arrangement, and while a project holds particles it gets a second arrangement, `SavedLayouts::for_a_project`, with the Effects panel in it. The row, an Effects panel, three commands (Add a particle pack…, Add particle effects… and New particle effects), a template and a goal card, with the empty state, the drop hint and French. An effect edit in that workspace is one step of Undo and a save of the document goes into the source (`an_effect_edit_is_one_step_of_undo_and_a_save_goes_into_the_source`).
- **Left alone, and reported.** The pin by Ctrl-click in the Effects panel, the Used by section (the references from a weapon, a vehicle or a ped to an effect are not built), a JSON schema for `particles.json`, the models as glTF (asked of the owner), the singleplayer DLC and `.oiv` places of a `.ypt` (not measured), and any claim that an effect plays in game. The JSON of a list is far larger than its file, and the largest, `core.ypt`, takes about 75 seconds to import on the test profile.

## Maps

The Maps view serves a person who makes a map: placements, the types they place, the interiors, the collision and the models and textures under them.

**What the person sees.** A task screen whose middle is a plan and not a model.

```text
+- Map - harbour -------------- build: FiveM resource [Legacy] ---------- [Problems 4] [Build] -+
| Project      | Maps and types        | Plan, seen from above           | Selection           |
|  Content     |  v <map>.ymap         |   +-------------------------+   |  entity             |
|   Map     2  |      entities         |   |  [] []  ::::   [] []    |   |  archetype          |
|   Files   1  |      LOD parents      |   |  boxes at their extents,|   |  position, rotation |
|  Builds      |  v <types>.ytyp       |   |  coloured by level of   |   |  LOD parent, flags  |
|   * harbour  |      archetypes       |   |  detail; an interior    |   |  [Uses] [Used by]   |
|              |      interior <name>  |   |  shows its rooms        |   |                     |
|  Uses        |  _manifest.ymf   ok   |   +-------------------------+   |  Interior           |
|   needs      |                       |  Layers [LOD] [Props] [Rooms]   |   rooms, portals    |
+-------------------------------------------------------------------------------------------+
| Problems | Build | History | Jobs | Log   <entity> parent out of range, <type> file not listed  |
+-------------------------------------------------------------------------------------------+
```

The left column has the Project panel and a tree of the map sets the project holds: a map name names a set of files that `MapGroup` merges (`crates/domain/or-world/src/group.rs`), so the tree lists sets, then files, then the entities, archetypes and interiors each holds, virtualised like the Assets table. The middle is a plan: each entity as a box at its extents, coloured by level of detail, with an interior drawn by its rooms, drawn by the egui painter so that it needs no streamed model. The right panel is the selection: the entity or archetype through the typed world views, with the reference pickers the inspector already has for archetypes and rooms, and a Uses and Used by pair from the dependency index. The bottom has the same tabs as the other task screens.

The 3D scene, placed models drawn where the map puts them, is the second stage and not part of the first cut. It needs the resolver to find each archetype's files and a streamer that loads them, and nothing in the tree measures how many models a map asks for; the plan view answers most questions a creator has before that.

**Empty state.** What the owner asked for: the Map view of a project with no map offers to add Map content or to open a `.ymap` or `.ytyp`.

```text
              No map in this project
   [ Add a map to this project ]   [ Open a .ymap or .ytyp… ]
   Files dropped here are added to the map. Import into the project decompiles a game map.
```

*Add a map to this project* gives the project an empty Map content, as *Add a clothing pack to this project* does for clothing. *Open a .ymap or .ytyp…* looks at a file where it is, in the plan, read only when it is the game's, and changes nothing: an opened file is a place to look in and not a member ([Open, Import and Add](Workbench_UX_Design.md#open-import-and-add)). Import game files… brings a game map in as a metadata source.

**Content kind.** `ModuleKind::Map`, written `map`, a folder with a manifest of map sets, each with a stable id and the files that make it: `.ymap` and `.ytyp` as `.metadata.src` folders (built, and the world family is named publicly, so they read with no snapshot), models and textures as their sources, collision kept whole until a bounds source exists, and the manifest the game needs. Most of what a Map content must know is in `crates/domain/or-world` already, and the kind is mostly an adapter over it. ⚠ The staged corpus holds no `.ymap` or `.ytyp`, so the metadata source of a map is checked with synthetic files (`a_ymap_in_each_binary_container_becomes_xml_and_an_edit_to_the_xml_compiles_into_it`) and the suites of the world model run over a mounted install, which the wave says when it reports what skipped.

| The kind says | It comes from |
|---|---|
| What a map set holds and what loads before it | `MapGroup` and the derivation of `_manifest.ymf` in `crates/domain/or-world/src/manifest.rs`; over a mounted Legacy install the derived lists match the shipped manifest in 8,964 of 10,603 maps (R32, #129) |
| What the engine trusts that the reader does not enforce: an index past its array, a cycle in the parent chain, a child count that is wrong, an interior's parallel arrays | `crates/domain/or-world/src/validate.rs`, the verb `openrage-cli world check` and the gate `write::refuse_if_broken` |
| How far a stored extent is from the recomputed one | `crates/domain/or-world/src/extents.rs` |
| Which type file declares an archetype, and whether its drawable exists | `crates/domain/or-world/src/archetypes.rs` and the resolver in `crates/domain/or-depends` |
| Archetypes made from drawables, in bulk | `openrage-cli world archetypes`, `crates/domain/or-world/src/generate.rs` (`what_an_archetype_takes_from_its_drawable`) |
| A map made and deleted without breaking a link | `crates/domain/or-world/src/execute.rs`; 9,113 Legacy and 9,079 Enhanced deletes keep every link (`executing_a_plan_keeps_every_link`, at `c33ae1c1`) |

What it contributes to a plan: the map files at `stream/` with the manifest lines that register type files, each archetype name and each map name as a claim (the model design already gives an archetype as its example of a name the game resolves once), figures for entities, archetypes and estimated memory, and an item per map set and archetype. The build reload extends to `.ymap`, `.ytyp` and `.ymf` by the hook above.

**Template.** *Map*, in the gallery already: content Map, one FiveM resource, opens on Map, pins Project, Map, Model, Texture and Data. *Props pack* is the same content with the archetype generator as its first action (see [props and interiors](#props-and-interiors)).

**Panels it reuses, and what it adds.**

| Reused as they are | Added |
|---|---|
| Project, Outline, Problems, Build, History, Jobs, Log; Uses and Used by (`src/ui/panels/hierarchy/uses.rs`) | A tree of map sets, files, entities and archetypes |
| The Visual editor and the reference pickers for rooms, archetypes and model sets (`src/state/reference_names.rs`) | A plan panel drawn by the egui painter, with layers |
| Presets, which apply one record's fields to a selection (`a_preset_from_one_record_applies_to_a_hundred_in_one_undo_step`) | A selection panel over the typed world views, and the rules as an adapter over `validate.rs` |

**Builds.** A FiveM resource ships it. A singleplayer DLC registers a type file as `DLC_ITYP_REQUEST` by its extension (measured over the retail packs, [Dlc_Pack_Creation.md](../features/Dlc_Pack_Creation.md#traps)); what streams a `.ymap` from a pack was not measured by any suite of this tree when this was drafted, and the generated pack's single change set enables files, so the cell stayed locked until a wave measured it. Wave P44 measured it ([what it found](#what-wave-p44-built-for-maps)): the install keeps its map files in nested archives that a `content.xml` registers, and the cell stays locked, for a type file as well as for a `.ymap`, since a type file the pack registers at `stream/` is not where the install keeps its own. An `.oiv` installs what the plan lists.

**Edited here, in a plugin, elsewhere.** In OpenRage: every field of an entity, archetype and interior, the references between them, the manifest, creating and deleting maps and entities with the links renumbered, bulk archetypes from drawables, and prefabs captured and placed by catalogue. In a plugin: markers and a thumbnail browser, the interior builder with portals and room bounds, and the lighting and environment tools, which [R47](Roadmap.md#r47-the-plugin-slate-after-the-first) files as separate applications, over the overlay and picking surface of [Plugin_ABI.md](../reference/Plugin_ABI.md). The draft put placing and moving with a picking, a gizmo and snaps there too, and the owner moved them into OpenRage (V5), where wave P44 built them. Importing Menyoo spooner XML is [R67](Roadmap.md#r67-menyoo-spooner-xml-import), unscheduled until a corpus exists. In another tool: modelling the buildings and props, and the sculpting of terrain.

**The bar.** The repository names CodeWalker, CodeX and OpenIV as the tools OpenRage replaces and lists, for the world editor, picking, gizmos and snaps, markers, a thumbnail browser and pawn modes, and for interiors portals, room bounds and rooms rebuilt from shell geometry (R47); that list is an inventory kept on the owner's machine and not in a clone ([Roadmap.md](Roadmap.md#inputs-outside-the-tree)), so this draft cannot say which tool has which. The bar is that the first group stays a plugin that can be written against the Map view's overlay, and that OpenRage holds itself to what it can check from the files alone: every map of a project checked at once for what the engine trusts, a manifest derived and compared, a delete that keeps every link, `openrage-cli world check` in CI, and a build that reloads each file it wrote.

**Not in the tree yet.** What [the subsection below](#what-wave-p44-built-for-maps) lists as left: the placed models of the 3D scene, a map made from nothing, an edit of a room or a portal, and a singleplayer DLC cell that says *Yes*.

### What wave P44 built for Maps

Steps M1 and M2 of [the order below](#the-order-it-could-be-built-in), the placement with a manipulator that the owner asked for in V5, and the measurement of where the install keeps its map files that V6 asked for before a cell says *Yes*. M3, the placed models in the scene, is not built: the scene draws the entities as boxes. The screen is described under [what the Map screen builds](Workbench_UX_Design.md#what-the-map-screen-builds), and the recipe was [what a kind of content costs](../reference/Crate_Reference.md#what-a-kind-of-content-costs).

- **What was measured, and where.** On the owner's Legacy install, 19,392 `.ymap` and 2,764 `.ytyp` files, and on the Enhanced install 19,956 and 2,877, each opens as its typed root (`CMapData`, `CMapTypes`) with none refused, and every member the typed records declare (`CEntityDef`, `CMloInstanceDef`, `CBaseArchetypeDef`, `CTimeArchetypeDef`, `CMloArchetypeDef`, `CMapData`, `CMapTypes`) is one the install states (`the_typed_world_family_names_only_what_the_install_states_and_the_rules_fire_on_few_of_its_maps`). The suites read `OPENRAGE_GAME_ROOT`, open the install read only and name themselves in the skip ledger when none is set, since the staged corpus holds no map file, and no game content is in the repository, a fixture or this document.
- **Where the install keeps its map files.** `where_the_install_keeps_its_map_files` lists every `.ymap` and `.ytyp` the mount holds by the device and folder of its logical path, and reads every `content.xml` the mount holds (123 on Legacy, 198 on Enhanced). A `.ymap` is in a DLC pack in 11,816 cases on Legacy and 12,385 on Enhanced, under `platform:/levels` in 4,586 and 4,581, under `update:/dlc_patch` in 2,773 on both, under `update:/x64` in 202 on both and under `platform:/dlcpacks` in 15 on both; a `.ytyp` is in a DLC pack in 1,814 and 1,943, under `platform:/levels` in 901 and 884, under `update:/dlc_patch` in 39 and 41, under `platform:/dlcpacks` in 9 and 8 and under `platform:/patch` in 1 on both. What a `content.xml` registers is a type file and an archive, and never a map file: `DLC_ITYP_REQUEST` names an `.ityp` 685 times on Legacy and 1,012 on Enhanced, and 684 and 1,010 of those are the `.ytyp` of a nested archive of the folder named, and `RPF_FILE` with `CONTENTS_DLC_MAP_DATA` registers an archive 312 and 471 times, the archive that holds the DLC's `.ymap` files. So a singleplayer DLC streams its maps from a nested archive that its `content.xml` registers, and the pack OpenRage generates writes no nested archive.
- **What the interiors state, and what stays carried.** An interior's rooms, portals, entity sets and time cycle modifiers are not typed in `or-io`: the typed `CMloArchetypeDef` carries them in the value tree it was read from, and a save writes them back as they came. The install states 11 members of a room (`attachedObjects`, `bbMax`, `bbMin`, `blend`, `exteriorVisibiltyDepth`, `flags`, `floorId`, `name`, `portalCount`, `secondaryTimecycleName` and `timecycleName`) on 2,149 rooms of Legacy and 2,184 of Enhanced, 8 of a portal (`attachedObjects`, `audioOcclusion`, `corners`, `flags`, `mirrorPriority`, `opacity`, `roomFrom` and `roomTo`) on 3,474 and 3,507, 3 of an entity set (`entities`, `locations` and `name`) on 2,276 and 2,281, and 6 of a time cycle modifier (`endHour`, `name`, `percentage`, `range`, `sphere` and `startHour`) on 1,243 and 1,269 (`the_typed_world_family_names_only_what_the_install_states_and_the_rules_fire_on_few_of_its_maps` prints them). They are measured and not declared: the rules read the counts and the indexes off the document, no edit of this wave touches a room or a portal, and no declaration of `or-io` changed, so declaring the four records, with the engine's names (D15), is the next step when the screen is to edit a room, and it is the wave that runs the or-io gates.
- **The content.** `or-maps` is the crate of [the shape above](#what-a-kind-of-content-hands-to-a-build) and `ModuleKind::Maps` its kind, `maps` in the project file, with `maps.json` in its folder: the map sets of the module, each with a stable id, the sources it is made of, its waivers and the names a person typed for the hashes a map keeps. `or_content::kinds` lists it, so the editor and `openrage-cli` plan it alike. A map file is a source of the metadata form (`<name>.metadata.src`, made by `or_source::metadata`), and a build ships its cooked file at `stream/`; a model and a texture dictionary ship there too. The reload hook that Views listed (`readers.rs`) reads a built `.ymap` and `.ytyp` as their typed root.
- **The rules.** Twenty-one, as data (`or_maps::RULES`, each a code, a severity and whether a waiver can set it aside). Errors, which no waiver mends: a source that leaves the folder, is missing or cannot be cooked, a file that does not read as a map or a type file, two sources cooking to one file, a LOD parent that is not in the map, a parent cycle or a child count the entities do not give, an interior that attaches an entity past its list or names a room it lacks, and a waiver with no reason. Warnings: two archetypes of the set with one name, an entity that places an archetype no type file of the set declares and the game does not know, two entities at one archetype, position and rotation, two entities with one guid, extents that do not hold the entities a map places, a bounding box with its minimum above its maximum, a source named by two sets or by none, and a waiver naming no rule. Notes: an archetype the set does not declare (the game or another pack may), a model no archetype names, an archetype whose asset no model holds, and a module with no map set. Which of them fire on the game's own files was measured, and a rule that fires on a tenth of the game's maps only warns: over 19,392 Legacy maps `entity-twice` fires on 8, `guid-twice` on 4, `archetype-undeclared` on 53 and `extents-short` on 939, and over 19,956 Enhanced maps on 8, 4, 48 and 994, and `hierarchy-broken`, `interior-broken` and `bounds-inverted` fire on none; `archetype-twice`, `archetype-without-model` and `file-twice` fire on the set and not on a map, since the measurement lays every type file and 2,000 maps into one set, so a name two archives hold collides, and no count of maps is claimed for them (`what_the_rules_say_of_the_install`, which holds `entity-twice`, `guid-twice`, `hierarchy-broken` and `archetype-undeclared` to a hundredth of the maps or fewer). `the_rules_fire_on_a_map_built_to_break_them` builds one map for each rule from synthetic files and finds each at its place.
- **Where a source ships, and the cells.** A cooked `.ymap` and `.ytyp` ship at `stream/`. A FiveM resource ships them. A singleplayer DLC is locked by `cannot_ship`, derived as ruled (V6) from what the content hands over: a `.ymap` is a file the pack is told no type for, and the sentence names it (`a_singleplayer_dlc_cell_is_derived_from_what_the_content_hands_over`), which is what the measurement above says should be the case, since the game finds a DLC's `.ymap` files in an archive that the DLC registers and the generated pack has none. A `.ytyp` that the manifest registers as `DLC_ITYP_REQUEST` outside every archive locks it the same way, in `cannot_ship` and in the plan of a singleplayer DLC alike, because 684 of the 685 `.ityp` names the install registers are the `.ytyp` of a nested archive and a pack with only type files has none (`a_pack_of_type_files_only_is_locked_for_a_singleplayer_dlc_too`). An `.oiv` ships what the plan lists. No build says a map loads in game, and the verifier says only that each source reached its place, that each file reads again as its root and that the manifest registers it.
- **Source edits.** `or_maps::edit` changes what an entity or an archetype says in the source: it cooks the source to bytes, opens the typed root, changes a member and saves the root, and `or_source::metadata::replace` writes the canonical XML back, refusing an edit whose XML does not compile back to itself and writing through a temporary file, so that a refusal leaves the source as it was (`a_pose_that_is_not_a_number_is_refused_and_the_source_is_as_it_was`). A move, a turn and a scale are stored the way the file stores them: a rotation as the conjugate quaternion for an entity and as it is for an interior instance, and a scale as `scaleXY` and `scaleZ` (`turning_and_scaling_are_stored_the_way_the_file_stores_them`). The typed world family cannot build a record from nothing without a schema snapshot, so an entity or an archetype that is added is a copy of one the file holds, with a guid or a name of its own, and an entity that is taken out renumbers the LOD links (`an_entity_is_added_as_a_copy_with_a_guid_of_its_own_and_taken_out_with_the_links_renumbered`). A pose or an entity written from the screen also grows the extents the map states, `entitiesExtents` and `streamingExtents`, to hold every entity whose archetype a type file of the set declares, in the same write and so the same step of Undo, and never shrinks them (`or_maps::edit::grow_extents` over the boxes the type files of the sets that name the source declare; `an_entity_moved_or_added_past_the_extents_grows_them_in_the_same_step`), because the game streams a map by them and `extents-short` would otherwise fire on any edit that moves an entity out of the box the file stated. Each of these edits is an operation of `or_maps::operation`, data with an inverse that the screen and `openrage-cli maps apply` apply through one function ([R70h](Roadmap.md#r70h-one-vocabulary-of-world-edits)). Not built: a map made from nothing, which wants the world family declared for a build from empty.
- **Placement.** The manipulator is `or_maps::gizmo`, pure maths over the scene, with no renderer: a ray through a pixel, the handle of an arrow, a ring, a plane square or the centre under it, the drag that moves, turns or scales along it, the pick of an entity by its box and the ground under it. It reaches the viewport as lines in the overlay list that plugins already draw through, in `src/render/map_overlay.rs`, so the renderer is untouched, and its keys and pointer are read by the scene panel. The scene is `or_maps::scene`, built over `or-world`'s placement rules (`Placement::Entity` stores the conjugate rotation and `Placement::InteriorInstance` stores it as it is, and a scale is `scaleXY` with `scaleZ`) with the box of each archetype, and the scan that reads the pack builds it with the rest. A person picks an entity in the list, the plan or the scene, drags an arrow or a ring, and one drag is one step of the pack's Undo that writes the source. Snaps are the grid, which the person sets in metres, a turn step in degrees, a scale step and the ground, which puts the entity on the highest box under it or on a height the person sets, and Ctrl held inverts the first. Every handle has a keyboard alternative: the position, the rotation about X, Y and Z in degrees and the two scales are fields that take what is typed, and the arrow keys, Page Up and Page Down and the brackets move and turn the entity selected by the grid step (`moving_an_entity_with_the_manipulator_writes_the_new_position_and_one_undo_restores_it`, `a_ring_turns_and_the_centre_scales_and_each_is_one_step`, `a_typed_position_and_the_arrow_keys_do_what_the_handles_do`). The plan is a second place to drag, along the ground.
- **Headless.** `openrage-cli maps list`, `validate` and `verify` (`a_map_pack_is_listed_and_validated_and_a_word_that_is_no_command_is_a_usage_error`, `a_build_of_maps_is_read_back_against_the_sets_and_a_tampered_one_fails`); the plan and the build are `openrage-cli project`.
- **The screen (M2).** One row of the table of task screens, a template (`Template::Maps`), a chip of the start page, nine commands and four panels (Maps, Scene, Plan and Selection), with its empty state, its drop and French. `the_maps_screen_on_screen` drives the screen through real frames, with a drag of the pointer on the manipulator, and every control is a row of [the control inventory](Workbench_UX_Design.md#control-inventory).
- **Questions the draft asked.** V5 is answered by the build: placement is in OpenRage, and the plugin route of [R47](Roadmap.md#r47-the-plugin-slate-after-the-first) stays available for what the app does not do, markers, a thumbnail browser, pawn modes and the interior builder. V6 is answered for maps: the places are measured, the cells stay derived, and the singleplayer DLC stays locked with its reason.
- **Left alone, and reported.** The placed models of M3, a map made from nothing, an edit of a room, a portal or an entity set (they are carried and not typed), the references from a map to the models it needs through the dependency index, a name resolver for the finding messages (a map keeps a hash, and the app shows the names the index and the pack hold), the singleplayer DLC cell, which wants the nested archive written, and any claim that a map loads in game.

## The other views, ranked

The criterion is what a project gains by holding the kind, set against what the kind costs, read from the tree. Four questions, each answered from a path.

1. **Is there something only a project can check?** A name the game resolves once, a reference from one file to another, a total across files. Without one a view is a better file list. This question ranks.
2. **What is declared and edited already?** A kind whose files OpenRage reads, writes and previews costs a screen, and one it only reads costs a model.
3. **What waits on something unbuilt?** A typed family, a source form, another view.
4. **What can nobody here verify?** What the game does with the file.

The ranking is by value. Question 4 has one answer for every row, that nobody here can run the game, which is why no build cell below says more than that a plan is valid. The order they could be built in is different, because cost moves some up ([the order](#the-order-it-could-be-built-in)).

| Rank | View | A project can check | Already in the tree | Waits on |
|---|---|---|---|---|
| 1 | Vehicles | A model name and a handling name the game resolves once; the files a vehicle needs | Fragment sources, vehicle glass kept on save, the shared dictionaries a vehicle opens, two variation roots named in `root_types.rs` | Typed vehicle metadata |
| 2 | Props and interiors | Every archetype has its drawable, texture dictionary and collision; rooms and portals refer to real entities | The whole of `or-world`, the archetype generator, reference pickers for rooms | The Map content |
| 3 | Audio | Which container holds a sound; a sound item that names a wave nobody ships | `.awc` open, play, export and replace; typed `.rel` items; `AudioIndex` | An `.audio.src` and `.rel` creation |
| 4 | Animations | A clip's bones resolve on a skeleton | The Animation workspace, clip table and a re-encoding writer | A `.clips.src` |
| 5 | Graphics packs | A texture replaced is the texture the game names | A parsed timecycle the renderer draws from | A writer for the timecycle, a scene |
| 6 | Scripts | The manifest names every script | The Files content, the Data workspace's text editor | Nothing, and a template and highlighting are the whole gain |

Each is reached by the doors of [how a view is reached](#how-a-view-is-reached), and each lists what it would hold in the same order as the three above: the figure, the kind of content, the template, what it reuses and adds, its builds, its empty state and where it is edited.

### Vehicles

```text
+- Vehicles --------------------------------------------------------------------+
| Project | Vehicles             | Assembled vehicle           | Vehicle            |
|         |  > <vehicle>    ok   |  body, wheels and doors     |  name, id          |
|         |    livery <a> <b>    |  from the fragment          |  model, handling   |
|         |    extras            |  Variation [livery][extra]  |  variations        |
|         |  > <vehicle>     !   |  Glass [x]  Skeleton [x]    |  [Info][Parts]     |
+-------------------------------------------------------------------------------+
```

A vehicle is a fragment with its physics children, its textures and its metadata. The tree has the hard parts of the model side: a fragment source whose glb replaces the common drawable and leaves the wheels and the physics to the base (`every_corpus_yft_extracts_and_compiles_back_to_the_same_drawable`), the vehicle glass kept through a save (`a_saved_yft_keeps_its_names_and_its_glass`), and the dictionaries every vehicle shares, opened only for a vehicle (`a_vehicle_opens_the_dictionaries_every_vehicle_shares_only_while_a_name_is_still_missing`). It lacks the metadata: `root_types.rs` names two vehicle variation roots among the `.ymt` roots, and no family declares the vehicle's own data. The owner calls vehicles the biggest kind after clothing, maps and weapons (WAR-210).

- **Content kind.** `vehicles`, a folder with a manifest of vehicles, each with a stable id, the model name the game resolves once (a claim), its files and its variations; the metadata is a typed family declared first, as for weapons.
- **Template.** *Vehicle pack*: one FiveM resource, opens on Vehicles, pins Project, Model, Texture and Vehicles.
- **Reuses.** The Project panels, the Model viewport with its fragment and skeleton, the Texture workspace and its rules for a livery, the Visual editor and presets.
- **Adds.** A list, an assembled-vehicle panel with a variation picker, a Vehicle panel, rules, a verifier and verbs.
- **Builds.** As Weapons.
- **Empty state.** *No vehicles in this project*, with *Add a vehicle pack to this project*, *Add vehicles…* and *Open a vehicle file…*.
- **Edited here, elsewhere.** Here: the data, the liveries by replacement, the body's geometry through the fragment source. Elsewhere: modelling, rigging, the sound and how it drives.
- **The bar.** The repository records nothing of how any tool handles a vehicle pack, so the bar is the five checks of the Weapons bar.

### Props and interiors

Not a kind of its own: a prop is an archetype and its drawable, and an interior is an archetype with rooms, portals and entity sets, all in `.ytyp` files that the Map content holds. The screen is the Map view with the Interior layer on.

```text
+- Map - corner_shop --------------------------------------------+
| Project | Types             | Plan, Interior layer  | Selection |
|         |  v <types>.ytyp   |  +----------------+   |  room     |
|         |    <prop>    ok   |  | room  | room   |   |  portal   |
|         |    interior       |  |   portal       |   |  entities |
|         |      <room>       |  +----------------+   |           |
+-----------------------------------------------------------------+
```

- **Content kind.** The Map content.
- **Template.** *Props pack*: the Map content with the archetype generator as its first action, one FiveM resource, opens on Map.
- **Reuses.** All of the Maps view, and the Model and Texture workspaces for the props themselves.
- **Adds.** An *Interior* layer in the plan, and the generator as a button: `openrage-cli world archetypes` takes bounds from the drawable and asks for the level-of-detail distance and the texture dictionary that no drawable records (`crates/domain/or-world/src/generate.rs`).
- **Builds.** As Maps.
- **Empty state.** The Map one, with *Add props…* (drawables and textures, the archetypes generated) in place of the second button.
- **Edited here, elsewhere.** Here: archetypes, interiors' rooms and portals as data. Elsewhere, in a plugin: building an interior from shell geometry (R47). Elsewhere, in another tool: the models.

### Audio

```text
+- Audio -----------------------------------------------------------+
| Project | Sounds                        | Stream               |
|         |  > <bank>.awc                 |  name hash, codec    |
|         |      <stream>      [Play]     |  rate, length        |
|         |  > <item>.rel   names a wave  |  [Replace] [Export]  |
+-------------------------------------------------------------------+
```

The tree opens an `.awc` as its own kind with a stream list, playback, WAV export and replacement of a stream from a 16-bit PCM WAV (`src/audio.rs`), reads the audio metadata as typed items (`crates/io/or-io/src/rel/game_object.rs`) and answers which container holds a sound (`crates/domain/or-depends/src/audio.rs`, `openrage-cli awc which`). A weapon's sound settings and a vehicle's are `.rel` items, so Audio is the second half of both and arrives after them.

- **Content kind.** None at first, as for Particles: the `.awc` and `.rel` files are compiled files kept whole. Later `audio`, with an `.audio.src` of WAV files and a stream table (planned), once a fixed point is measured.
- **Template.** *Audio pack*: the Files content, one FiveM resource.
- **Reuses.** The `.awc` inspector and player, the typed items, the audio index and the Project panels.
- **Adds.** A Sounds panel listing the banks and items of the project and checking that an item names a wave some bank of the project or the game has, and a layout, with the same saved-layout trap as Particles.
- **Builds.** FiveM as files; a singleplayer DLC is locked, because the types a pack registers audio files as are not recorded in this tree.
- **Empty state.** *No audio in this project*, with *Add audio…* (`.awc` files and WAV files), *Import game files…* and *Open an .awc…*.
- **Edited here, elsewhere.** Here: replacing a stream and the items that name it. Elsewhere: recording, mixing and sound design.

### Animations

The Animation workspace plays a clip bound to a skeleton from another document and writes edited channels back ([Animation_Preview.md](../features/Animation_Preview.md)), and a clip dictionary in a project is a compiled file kept whole that the Assets table already opens there. The gain of a view is small against what a layout of the existing workspace gives, so it follows the recommendation for Particles: a panel and a template first.

```text
+- Animation - emotes ------------------------------------------+
| Project | Clips                | Viewport: the skeleton        |
|         |  <dict>.ycd          |  playing the clip, bones shown|
|         |   > <clip>     ok    +-------------------------------+
|         |   > <clip>      !    | Timeline and curves           |
+----------------------------------------------------------------+
```

- **Content kind.** None at first. Later `clips`, glb bone tracks and a clip table (planned), when a fixed point is measured.
- **Template.** *Animation pack*: the Files content, one FiveM resource, opens on Animation.
- **Reuses.** The Animation workspace: its viewport, clip table and preview settings.
- **Adds.** A Clips panel listing the clips of every dictionary of the project and whether each resolves on a skeleton.
- **Builds.** FiveM as files; a singleplayer DLC is locked, as for Particles.
- **Empty state.** *No animation in this project*, with *Add clip dictionaries…* and *Open a .ycd…*.
- **Edited here, elsewhere.** Here: a clip's channels and a bone's numbers. Elsewhere: authoring a motion.

### Graphics packs

The tree parses the time table and the weather cycles of an install (`crates/io/or-io/src/gta5/data/timecycle.rs`), and the renderer draws the sky, the lighting and the tone from them (`crates/runtime/or-renderer/src/game_shaders/lighting.rs`). A value can be set in memory and nothing writes a file back. A graphics pack is three things: timecycle and weather data, replaced textures, and the visual settings. [Wardrobe_Parity.md](Wardrobe_Parity.md) calls graphics packs a parked label with no design, and this section is the design's first lines and not a plan.

```text
+- Graphics - harbour_look --------------------------------------+
| Project | Files               | Viewport: sky and light        |
|         |  <weather>.xml      |  Time [-----|----] Weather [v] |
|         |  <texture>.ytd      +--------------------------------+
|         |                     | Keyframes: a value over a day  |
+-----------------------------------------------------------------+
```

- **Content kind.** `graphics`, later: the data files with a writer and a canonical form that reaches a fixed point, and the textures as `.textures.src`.
- **Template.** *Graphics pack*, offered only once a writer exists.
- **Reuses.** The Texture workspace and its sources, and the renderer with the Time and Weather controls of the viewport.
- **Adds.** A writer for the timecycle, an editor for its keyframes and a scene panel in which an edit is seen.
- **Builds.** FiveM as files; a singleplayer DLC is locked.
- **Empty state.** *No graphics in this project*, with *Add replacement textures…* now and *Add a timecycle…* when it can be written.
- **Edited here, elsewhere.** Here: the textures, and the values once they can be saved. Elsewhere: nothing in this draft.

### Scripts

Scripts ship as the Files content. A script cannot ship in a singleplayer DLC and the cell says so (`a_script_cannot_ship_in_a_singleplayer_dlc_and_the_reason_names_it`), the text editor highlights XML, JSON, INI and IDE or IPL lists and has no Lua, JavaScript or C# language (`Language` in `src/ui/panels/text_editor/highlighters.rs`), and a compiled `.ysc` opens as a read-only listing (`a_script_opens_as_its_listing_and_says_why_natives_are_hashes`). OpenRage has no edge in scripts, so the proposal is a template and highlighting, and no workspace. The screen is the Project workspace.

```text
+- Project - my_scripts ----------------------------------------+
| Content        | Assets                   | Properties       |
|  Files         |  <script>.lua    Ready   |  type, size      |
|  Builds        |  <script>.js     Ready   |  Open in > Data  |
|   * my_scripts |                          |                  |
+----------------------------------------------------------------+
```

- **Content kind.** None: the Files content.
- **Template.** *Script resource*: the Files content and a FiveM resource, opens on Project.
- **Reuses.** The Project workspace and the Data workspace's text editor.
- **Adds.** Highlighting for the languages the template names.
- **Builds.** FiveM; a singleplayer DLC is locked, because it runs no script.
- **Empty state.** The Project workspace's own: *Add files…* and the line that files can be dropped.
- **Edited here, elsewhere.** Here: the text of a file. Elsewhere: running, debugging and the scripting runtimes.

## Builds each view can ship in

The cell of a view and a kind of build is derived, not declared: `cannot_ship` reads what the content contributes against the kind of build and gives the sentence the Builds window shows. The recommendation is to keep it derived (ruling V6).

| View | FiveM resource | Singleplayer DLC | `.oiv` package |
|---|---|---|---|
| Weapons | Yes | When every metadata file states a type and every model sits in an archive folder | Yes, laid out as a pack |
| Particles | Yes | Locked: a `.ypt` has no type and no place measured | Locked until the place is measured |
| Maps | Yes | Locked: a `.ymap` has no type and a `.ytyp` would sit loose, and the install keeps both in nested archives that a `content.xml` registers (measured, wave P44) | Yes, as a pack |
| Vehicles | Yes | As Weapons | Yes |
| Props and interiors | As Maps | As Maps | As Maps |
| Audio | Yes | Locked: the types of audio files are not recorded | The same question |
| Animations | Yes | Locked, as Particles | The same question |
| Graphics packs | Yes, as files | Locked | The same question |
| Scripts | Yes | Locked: a DLC runs no script | Files at the paths the plan lists |

Nobody here can run GTA V, so a cell that reads *Yes* means the plan is valid and every file reloads, and a cell that reads *Locked* means the tree measured nothing that lets the plan claim a place.

## The order it could be built in

Nothing here is scheduled; the orchestrator turns an approved step into a wave. The three views share no file except the table of task screens, so after step 0 they can run in separate lanes.

| Step | What it builds | Ends on a check that fails first |
|---|---|---|
| 0 | The table of task screens, with the Wardrobe moved onto it. **Built (wave P40).** | The Wardrobe suites pass unchanged, and a test row for a stand-in task screen is offered only while its content is held and appears in Open in with its reason (`a_second_task_screen_is_one_row`, and the screens before and after in `the_task_screens_draw_as_they_did_before_the_table`) |
| W1 | **Built (wave P42).** The weapon family in `or-io`, measured on a mounted install | Every weapon file of both installs reaches the canonical fixed point; a record built from nothing round trips; a field reads under the engine's name with no snapshot |
| W2 | **Built (wave P42).** The Weapons content crate, rules, plan, verifier, verbs and the reload hook | A weapon pack plans and builds; a name held twice and a missing model are refused; every file read back |
| W3 | **Built (wave P42).** The Weapons workspace, the assembled-weapon query, the template and the empty state | The self-check draws the screen; the query selects what the pack states, with no renderer |
| P1 | **Built (wave P43).** The Effects panel, the Particle pack template and its layout | The panel lists the effects of every `.ypt` of a project; the self-check draws it |
| P2 | **Built (wave P43).** A `.ypt` source and a content kind | Every `.ypt` of both installs reaches a fixed point through the text form (`every_ypt_of_the_install_reaches_a_fixed_point`, install-gated) |
| M1 | **Built (wave P44).** The Map content and its rules, and the reload hook for map files | `openrage-cli project build` plans and verifies a project with a map, and a broken parent index stops it |
| M2 | **Built (wave P44).** The Map workspace, the plan and scene panels with the manipulator, the template and the empty state | The self-check draws it; a drag of the manipulator on the scene writes the source and is one step of Undo |
| M3 | The 3D scene of placed models. Not built: the scene of M2 draws each entity as a box | Measured separately, after M2 |
| Then | Vehicles, props and interiors (a template and a layer once M2 stands), audio, animations, graphics packs and a script template | Each ends as the steps above do |

## Rulings the draft asks of the owner

**Answered 2026-10-20 on the decision sheet.** Accepted as recommended: Q1, Q2, Q4, R8, V1, V2, V3, V6 and V7. Changed by the owner: V4, Particles get a source form and a content kind (not only a panel, a template and a second layout), and V5, the Map view includes placement of objects with a manipulator in OpenRage (not only a plugin). The sections below keep the first recommendations as the history of the draft; waves P42 (Weapons), P43 (Particles) and P44 (Maps) of [Workbench_Plan.md](Workbench_Plan.md) rewrite them as they build.

Q1, Q2 and Q4 are questions of [Wardrobe_Parity.md](Wardrobe_Parity.md#questions-for-the-owner) that concern the kinds of content, and the draft answers them with a recommendation. R8 is a ruling of the second plan, and V1 to V7 are new. Each stays at the default until a sentence from the owner decides it.

- **Q1, which kind is next, and in what order.** Recommended: the owner's order of the views, weapons, particles and maps, then vehicles, props and interiors, audio, animations, graphics packs and scripts, with this draft standing as the step before any of them. Each kind starts with its data (a typed family, a source form, a measurement) and gets no screen before it. Two sentences of the owner can be read as orders, clothing then addon peds then vehicles (WAR-215) and weapons then particles then maps (PRO-180); the draft reads the first as the order of kinds the project system was built for, which clothing and addon peds met, and the second as the order of these views.
- **Q2, the form of a metadata source.** Recommended: the folder that is built, with the text `.meta` row added (V3).
- **Q4, other platforms.** Recommended: Windows only for every view. A source says win64 and a build refuses a source of another platform, so none of the new kinds states a platform, and the console layouts stay [R36](Roadmap.md#r36-console-and-platform-variants)'s.
- **R8, the workspace a project opens on.** Recommended: Project when the project holds several kinds of content, and the kind's own screen when it holds one. It is built (`workspace_for`), and each new view would join it unchanged. The owner is asked to confirm it as a ruling.
- **V1, the conflict of the two orders above.** Recommended: weapons first, as in PRO-180, the later and more specific sentence, and vehicles first among the others.
- **V2, a table of task screens before the second one.** Recommended: yes. One row for each task screen holds its workspace, its kind of content, the heading of its empty state and the action that adds the content, its layout and the word its count is said in. The places in [the table above](#what-a-view-costs-in-the-tree-today) read it, and the Wardrobe moves onto it first.
- **V3, where a data source ships.** Recommended: at the path its content's table states, so a text `.meta` can be a source of a Weapons or a Vehicles content. Accepted, and stated in the row: `data_sources` of `TaskScreen` is the folder of a resource, ending in `/`, at which a build registers a data source of the content by that path; the Wardrobe's row states none, since a clothing pack keeps no text `.meta` of its own. The wave that builds a Weapons content states its folder from a measurement and reads it in the plan. Built (wave P42): the folder is `data/` and the plan reads it from the row (`DATA_FOLDER`); it is a choice and not a measurement, since FiveM reads a data file by the path its manifest names, and what was measured is the type each root registers as.
- **V4, Particles with no content kind and no source form for now.** Recommended: yes, a panel, a template and a second saved layout for a project; the `.ypt` stays a compiled file kept whole (ruling R3 of the second plan) until a text form is shown to reach a fixed point and a rule needs a kind. Changed by the owner on 2026-10-20 and built (wave P43): a source form and a content kind as well.
- **V5, the Map view's reach.** Recommended: inspection, rules, creation, deletion and packaging in OpenRage, and placement with a gizmo as a plugin (R47); Menyoo import stays R67. Changed by the owner on 2026-10-20 and built (wave P44): placement with a manipulator is in OpenRage, and the plugin route stays available for what the app does not do.
- **V6, build cells.** Recommended: keep them derived by `cannot_ship`, and measure the places a `.ypt`, a `.ymap` and an audio file have in the retail packs before any cell says *Yes*. Kept derived; wave P44 measured the places of a `.ymap` and a `.ytyp` on both installs, and the singleplayer DLC cell of a map stays locked with its reason.
- **V7, the Weapons view's reach.** Recommended: the weapon's metadata, models, textures and components with their attachment points, and effects, sounds and animations as references that open their own workspaces and not as content of the kind. Built (wave P42) as recommended: the Weapon panel's Effects, sounds and animations name what the weapon states, each with an Open into Particle, Data or Animation.

## What this draft does not do

It adds no code, no kind of content, no workspace, no template and no panel, and it does not edit the wave table or the status of [Workbench_Plan.md](Workbench_Plan.md). It builds nothing held back: no CodeWalker XML, no author mode, no shipped presets, and no claim that a build loads in game. It does not measure the places an asset has in a retail pack, the weapon family or the cost of a 3D scene, and it says where each of them waits. The text of [R44f](Roadmap.md#r44f-payload-and-gpu-access) and [R46](Roadmap.md#r46-first-plugin-a-clothing-tool-on-openrage) is not edited here.

## Sources

[Workbench_Plan.md](Workbench_Plan.md), [Workbench_UX_Design.md](Workbench_UX_Design.md), [Project_Model_Design.md](Project_Model_Design.md), [Source_Format_Design.md](Source_Format_Design.md), [Clothing_Tool_Design.md](Clothing_Tool_Design.md) and [Wardrobe_Parity.md](Wardrobe_Parity.md) for the rulings and the model; [Particle_Preview.md](../features/Particle_Preview.md), [Animation_Preview.md](../features/Animation_Preview.md) and [Dlc_Pack_Creation.md](../features/Dlc_Pack_Creation.md) for what the tree measured; [R32](Roadmap.md#r32-world-data-ymap-ytyp-and-interiors), [R34](Roadmap.md#r34-audio-and-media), [R47](Roadmap.md#r47-the-plugin-slate-after-the-first) and the tree at `e5d444fd` for the rest; `docs/audit/workbench-open-2026-10-20.json` for the owner's requirements, and a listing of the staged metadata corpus taken on 2026-10-19.
