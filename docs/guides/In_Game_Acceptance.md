# In-game acceptance

What a person looks for in a running game to say that GTA V loads a file OpenRage wrote: the sheet of lines, the recipe that makes a build to check them from your own install, how to load that build, and how to write down what you saw.

## What the sheet is, and is not

Nothing in OpenRage runs the game. A build reads every file it wrote back through OpenRage's own readers and says so, and whether the game loads them is something only a person at a running game can see ([who runs the game](../direction/Workbench_Plan.md#held-back)). The sheet is where that is written down.

- **It is one file,** `crates/domain/or-project/src/acceptance.json`, which the code reads (`or_project::acceptance`) and this guide lists. A line is one thing to do in the game and one thing to look for there, about one kind of file a build writes, with the row of [the roadmap](../direction/Roadmap.md) that writes that kind. Under a line are its results: each is one run by a person, with the edition, the game's build, where it was loaded, the day and the verdict.
- **It holds no result that nobody saw.** A result is written by the person who ran the game. A build, a test and an agent write none, and a line with no result is not checked in game, which is all that `openrage-cli project acceptance` says of it.
- **A build says what the sheet holds of it.** Under what a build read back, the Build tab, `openrage-cli project build` and `openrage-cli wardrobe build` count the kinds of file it wrote that a line is about and that nobody saw the game load, for the edition the build is for and for both when it states none, and name them; when every such line was last seen to load they say so, with the day of the newest result. They say what was seen and never that a file loads. ⚠ A kind is named until every line about it has a newest result of loaded, and a line is matched by the kind of file alone: maps are named until `map-made-from-nothing` has one too, whatever maps the build holds. The tab's link opens this guide where a build is loaded, and Help ▸ In-game acceptance guide opens it from the top.
- **It is not a sample.** The repository holds no project to load: a changed map of the game holds the game's content, which no commit takes. The sample is [a recipe](#make-the-sample) you run over your own install.
- **A result is about one day.** It says what one build of the game did with one build of OpenRage's. The newest result of a line for an edition is the one that stands, so a line that loaded and later did not reads as not loaded.
- **A line is about the game taking a file,** and no proof that every value in it is honoured: the object is drawn where the map puts it, or it is not.

## The lines

The table is the data file's lines, one for one (`the_guide_lists_the_lines_of_the_sheet`); the kind is the extension of the built file.

| Line | Kind | Row | In the game | Look for |
|---|---|---|---|---|
| `map-object-moved` | ymap | [R32](../direction/Roadmap.md#r32-world-data-ymap-ytyp-and-interiors) | Go to the place where the object you moved stood. | The object is drawn at the place you moved it to, and nothing is drawn where it stood. |
| `map-object-added` | ymap | [R62](../direction/Roadmap.md#r62-the-rbm-layout-synthesiser) | Go to the place where you added an object as a copy. | The copy is drawn where you put it, and every object the map placed before is still drawn. |
| `type-file-read` | ytyp | [R32](../direction/Roadmap.md#r32-world-data-ymap-ytyp-and-interiors) | Go to an object of the archetype you changed in the type file. | The object is drawn with its model and its textures, as it was before the build was loaded. |
| `map-made-from-nothing` | ymap | [R70h](../direction/Roadmap.md#r70h-one-vocabulary-of-world-edits) | Go to the place of the object in a map you made from nothing. | The object is drawn where the map places it. |
| `story-pack-mounts` | rpf | [R51c](../direction/Roadmap.md#r51c-dlc-pack-creation) | Install the pack, add its line to the DLC list and load a save in story mode. | The game reaches the story with no error, and what the pack adds is in the game. |

A line names a kind of file a build reads back, so a part of [R70](../direction/Roadmap.md#r70-the-gta-v-world-editor) that makes a build write a new kind adds the reader, then its line, then the step of [the recipe](#make-the-sample) that makes a file of that kind, in its own change: `the_sheet_and_the_recipe_are_about_the_same_kinds_of_file` fails for a line whose kind the recipe does not build, and for a kind the recipe builds that has no line. These cannot be made yet:

- **A map made from nothing** has its line, `map-made-from-nothing`, and no step of the recipe: `openrage-cli world new-map` writes one, as PSO where the game's own maps are RBM ([R70h](../direction/Roadmap.md#r70h-one-vocabulary-of-world-edits)), and the step that brings it into the sample is not written. A type file made from nothing has no line, and an object moved past the extents its map stated has none either.
- **The far model of a removed object** waits for [R70k](../direction/Roadmap.md#r70k-removing-what-the-game-placed), the wall nobody sees for [R70l](../direction/Roadmap.md#r70l-editing-collision) and the hole in the occlusion for [R70o](../direction/Roadmap.md#r70o-parked-vehicles-zones-lights-and-occluders), which also brings parked vehicles, zones and lights.
- **A changed collision file and a changed path region** ship from the sources a project now holds them as ([R70w](../direction/Roadmap.md#r70w-a-changed-bound-and-a-changed-path-region-as-sources)), and have no line and no step of the recipe; roads and scenario points are edited by [R70p](../direction/Roadmap.md#r70p-roads-scenario-points-and-the-navmesh).
- **A room, a portal and a second instance of an interior** wait for [R70m](../direction/Roadmap.md#r70m-interiors), and a door that opens for [R70n](../direction/Roadmap.md#r70n-doors).
- **The sound that stops at a wall** waits for [R70x](../direction/Roadmap.md#r70x-the-audio-of-an-interior), which writes the audio game data whose edit [R34b](../direction/Roadmap.md#r34b-the-audio-metadata-schemas) leaves open.
- **The manifest of a set of maps** is written by a Maps content that keeps a manifest of the game ([R70q](../direction/Roadmap.md#r70q-a-build-a-server-and-a-story-install-take)), and has no line and no step of the recipe, whose sample is Files content and writes none. **A map in a story pack** is written by the same row, and its build is still refused.
- **A navmesh polygon the game skips** has no part: whether the game skips a polygon that `openrage-cli navmesh disable` marked is the open clause of [R33e](../direction/Roadmap.md#r33e-navmesh-index-stability), no build writes that file and no part of R70 authors a navmesh.

The files other kinds of content write (textures, models, clothing, weapons, particles) have no line yet. The command names each kind of file of a build that the sheet says nothing about.

## Make the sample

You need your own GTA V install, `openrage-cli`, and a scratch folder outside every repository. `<install>` is the install's folder and `<scratch>` the scratch folder below; `<model>`, `<map>` and `<types>` are names you find in step 1.

⚠ What these steps make holds the game's content: the sources taken from the game and the build made of them. Keep the folder out of Git and send it to nobody; `--from-game` marks the sources so that the project's own `.gitignore` leaves them out and `openrage-cli project check-sharing` names them.

1. **Choose an object.** Take a small prop that stands alone in a place you can walk to and whose model name you know.

   ```
   openrage-cli depends placements --game <install> <model>
   openrage-cli names --game <install> --resolve <hash of a map>
   openrage-cli depends of --game <install> <model>
   ```

   The first prints each map that places the model, by the hash of the map's name, with the position of each placement; the second gives a map's name for its hash; the third lists the files the install loads for the model, and the line marked `DeclaringTypeFile` is its type file. Keep one position.

2. **Take the map and the type file out of the install**, as the game would load them.

   ```
   openrage-cli bulk --game <install> --glob "**/<map>.ymap" --out <scratch>/game --flat
   openrage-cli bulk --game <install> --glob "**/<types>.ytyp" --out <scratch>/game --flat
   ```

3. **Make a project and import both.** `--game` is the edition of your install, `legacy` or `enhanced`.

   ```
   openrage-cli project create <scratch>/acceptance --name acceptance --resource or_acceptance --kind fivem --game legacy
   openrage-cli project import <scratch>/acceptance/acceptance.openrage-proj <scratch>/game/<map>.ymap <scratch>/game/<types>.ytyp --from-game
   ```

   Each file becomes a source folder under `sources/`, and what you edit is its `metadata.xml`.

4. **Move the object.** In `sources/<map>.metadata.src/metadata.xml`, find the `CEntityDef` item whose `position` is the one you kept, which the command printed to the centimetre. Its `archetypeName` is the model's name, or `hash_` and the hash that `openrage-cli hash <model>` prints. Take an item whose `parentIndex` is -1 and whose `numChildren` is 0, so that no far model stands in for it, and add two metres to the `x` of its `position`.

5. **Add an object as a copy.** Copy that whole `<Item>` and paste it after itself. In the copy, take two metres off the `x` the object had at first, and give `guid` a value no other item of the file has.

6. **Change the type file.** In `sources/<types>.metadata.src/metadata.xml`, find the archetype whose `name` is the model's and add one to its `lodDist`. A source that is still the game's own file is left out of a build, so a type file you do not change does not ship.

7. **Build, and ask what to look for.**

   ```
   openrage-cli project build <scratch>/acceptance/acceptance.openrage-proj
   openrage-cli project acceptance <scratch>/acceptance/build/or_acceptance
   ```

   The build writes `stream/<map>.ymap`, `stream/<types>.ytyp` and `fxmanifest.lua` into `build/or_acceptance`, reads the map and the type file back and says so. The second command prints every line above whose kind is a kind of file the build wrote, and for each edition its newest result or that it is not checked in game. ⚠ A line is matched by the kind of file alone, so the command prints four lines for this build: the three about a changed map and a changed type file, and `map-made-from-nothing`, though the sample holds no such map.

An edit made in the XML does not grow the extents the map states, and the game streams a map by them, so keep the object and its copy within a few metres of where the object stood. No verb of `openrage-cli` changes an entity of Files content, which the sample is; `openrage-cli maps apply` changes one of a Maps content ([R70h](../direction/Roadmap.md#r70h-one-vocabulary-of-world-edits)). The Map screen of a project with a Maps content makes the same two edits with its manipulator and grows the extents as it writes ([what the Map screen builds](../direction/Workbench_UX_Design.md#what-the-map-screen-builds)).

A project made by these commands holds its sources as Files content, whose build writes no manifest line for a type file; a Maps content writes the line that registers it. Whether a server needs that line for a type file that replaces the game's own is one of the things a result of `type-file-read` tells.

⚠ The resource these commands build does not declare itself a map: only the build of a Maps content writes `this_is_a_map 'yes'` into `fxmanifest.lua`, and `openrage-cli project check-resource <scratch>/acceptance/build/or_acceptance` warns of it as `map-not-declared`. A moved object that is not drawn may be that missing line and not the map file, so the commit that records a `not-loaded` from this sample says that the resource was not declared a map.

## Load it on a FiveM server

OpenRage does not start or change a server ([install on a server](Workbench_Guide.md#install-on-a-server)).

1. Copy the folder `build/or_acceptance` into the server's `resources` folder.
2. Stop every other resource that ships a file of the same name: a server that runs two resources shipping one map loads one of them, and you would not know which you looked at.
3. Add `ensure or_acceptance` to `server.cfg` and start the server, or type `refresh` and then `ensure or_acceptance` in the console of one that runs.
4. Read the server's console, and the client's, while the resource starts. An error that names a file of the resource is a fault to write down in its words.
5. Join, go to the position you kept, and do the three lines the sample is about: look for the moved object, the copy, and an object of the archetype you changed. Write no result under `map-made-from-nothing`, which the command printed too: the sample holds no map made from nothing, and a result under a line is shown to every build that wrote a file of its kind.
6. Type `stop or_acceptance` and look again. The object is back where the game puts it and the copy is gone, which says that what you saw came from the build.

## Load it in story mode

A build goes less far here, and OpenRage writes nothing into an install.

- **A story pack cannot carry the sample's map today.** A build of the kind `singleplayer-dlc` over the project above is refused with a sentence that names the two files, because the game streams a pack's maps from an archive inside the pack, and the sample's Files content hands them over outside one. A Maps content hands its maps over inside such an archive, and its build is refused too, for what nobody measured of that archive ([R70q](../direction/Roadmap.md#r70q-a-build-a-server-and-a-story-install-take)). The lines about a map and a type file are checked on a FiveM server until then.
- **What can be loaded is a pack of content a pack carries:** clothing, weapons, or files given a type. Declare the build with `openrage-cli project add-build <project.openrage-proj> --kind dlc`, build it, and the build writes `build/<name>/dlc.rpf` and prints the line `dlclist.xml` needs ([how a pack is written](../features/Dlc_Pack_Creation.md)).
- **Install it by hand**, in the `mods` folder of the install and never in the game's own files: the pack goes to `mods/update/x64/dlcpacks/<name>/dlc.rpf`, and the printed line into the `dlclist.xml` of the `mods` copy of `update/update.rpf`, under `common/data`. Settings ▸ Editing ▸ Mods overlay says whether the game will read `mods` from the files beside its executable; no run has confirmed that verdict either ([R49d](../direction/Roadmap.md#r49d-the-game-side-loader)), so a pack that loads from there is the first to do so.
- **Start the game, load a save** and do the line `story-pack-mounts`: the game reaches the story, and what the pack adds is there, such as a garment in a wardrobe.

## Write down a result

Open `crates/domain/or-project/src/acceptance.json`, find the line by its id and add one object to its `results` for each run:

```json
"results": [
  { "edition": "legacy", "build": "3258", "where": "fivem-server", "date": "2026-12-01", "verdict": "loaded" }
]
```

- `edition` is `legacy` or `enhanced`: the edition of the game you ran.
- `build` is the game's build as you read it, such as the build number the server enforces.
- `where` is `fivem-server` or `story-mode`.
- `date` is the day of the run, written `YYYY-MM-DD`.
- `verdict` is `loaded`, `not-loaded` or `loaded-with-fault`. The last needs a `fault` beside it, one sentence that says what was wrong; the first two take none.

Add a result and leave the older ones as they are: the sheet is a record of runs. Then check the file and read it back, from the tree:

```
cargo nextest run -p or-project -p or-cli -E 'test(/acceptance|sheet|recipe/) + binary(/acceptance/)'
cargo run -p openrage --bin openrage-cli -- project acceptance <scratch>/acceptance/build/or_acceptance
```

The first runs every test that reads the sheet, in `or-project` and in `openrage-cli`, and refuses a result with a verdict outside the three, a day that is no day of the calendar or a key it does not know, naming the line by its id. The second prints your result under the line, for its edition, and leaves the other edition not checked in game. ⚠ The command reads the sheet it was built with, so an `openrage-cli` built before your edit prints the sheet as it was; `cargo run` builds it again. Commit the file alone, with what you ran in the message; when a line answers the open clause of a row of the roadmap, say so in that row.
