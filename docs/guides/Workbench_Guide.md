# Workbench guide

How to make clothes in OpenRage if you come from Durty Cloth Tool: add your clothes, fix what is wrong, build it, put it on a server, and send a project to someone without sending what is not yours. The app shows this guide under Help ▸ Guide, and the start page links into it.

## Try it first

No pack of your own yet? Help ▸ Open the sample project makes a small clothing project from synthetic content, kept beside your preferences, with one problem to find: two jackets that ship the same model. Help ▸ Take the tour walks six screens in a corner window and ends on this guide. Both are on the start page too.

## Add your clothes

You have clothes: models (`.ydd`), textures (`.ytd`) and perhaps cloth (`.yld`), named the way the game names them.

- Start page, New project…, then Clothing pack. Name it, choose a folder and press Create. The project opens on the Wardrobe.
- Drop the files or the folder on the Wardrobe, or press Add clothes…: each model becomes a garment. The ped and the slot come from the file names, and a name that does not say them leaves the garment marked `?` until you set them.
- Nothing you drop is changed: your files stay where they are. Each model and texture becomes a source in the project, made from your file at once, and the toast that says what was added has an Undo button that takes that drop back.
- If the pack is a folder you already keep somewhere, use Open folder… on the start page and press Make this a project on the banner. It writes a project file and moves nothing.

There is no project type to choose. A project holds content, and a clothing pack is one kind of content; the same project can hold loose files beside it.

## See what is wrong and fix it

Problems, in the Project workspace and in the line under the Wardrobe's list, lists what the rules found, one sentence each. An error stops a build that ships it and a warning does not, and every screen counts them by those two words: the Content rows, Properties and Problems say `1 error, 18 warnings`, while a build says what stops it (`3 errors stop this build`).

- Fix… says exactly what it will change, then does it.
- Waive… asks why the finding stands and records the reason in the pack, so it stops counting.
- A texture or a cloth the pack names and cannot find offers Locate… and Remove this texture or Remove the cloth, which take the name off the garment and leave the garment where it is.
- Fix all safe, in the header of the Wardrobe, does the fixes that change no picture on purpose and take no garment out, in four steps that each report in the Log: it repairs a model whose counts contradict their runs where that loses nothing (a protected model, one that opens with `FXAP`, is refused by its name), gives a texture its mips, the nearest power of two and BC1 or BC3 (keeping what a rule the project switched off or waived is about, and saying which rule kept it), lets go a pin that clashes with another, and states a drawable's mask again where only its skin bit disagrees with the garment's skin (a mask that asks for the match form stands). It lists what it leaves and why, it is one Undo, and pressed again it changes nothing. A box under it in the Fix menu, Also add missing levels of detail, makes it do one more thing in the same Undo: give each model that has a high level and no medium or low one those levels. It is off to begin with, because a level is a change to a model's geometry.
- Optimise textures… makes the textures of the garments you picked, or of the whole pack, lighter: the same fixes and, if you choose, half or a quarter of the size or a longest side. The texture source folder is not changed; the garments name a lighter copy of it, and Undo takes the copy away. A texture that is a compiled `.ytd`, a picture or a reference is listed and left as it is.
- Make normal and specular maps… lists the maps of each diffuse texture (a name with the word `diff`) and makes them as new pictures in a copy of the texture's source, which the garments name; they are an estimate from the brightness of the diffuse, and a map the source has already is left. Undo takes the copy away.
- Budget textures… asks for the memory the textures should fit and plans it: each texture gets the size and format that lose the least picture per byte saved, with the PSNR of the choice and the texture before and after side by side. Nothing is written until Accept, which names lighter copies; Undo takes them back.
- Complete materials states the constants a shader declares that a material of the models lacks, at the values the shader declares (it needs the shader database of an install). A sampler the material lacks is left.
- Watched folders, a fold of Details below the textures, binds a folder your image editor saves into to the diffuse, specular or normal slot of a texture; what is saved there lands in the texture's source and the preview is drawn again. A 16-bit picture is given the rule that keeps its whole range, in the source's `textures.json`, where the inspector shows it.
- Repair model, beside Replace model, does the repair of a single garment on request.
- The Mesh doctor, a fold of Details below Files, works on the models of the garment you selected. Check mesh only looks: the triangles of each level of detail, the levels the model lacks, a rig that names a group the skeleton does not know, weights that are noise, a shoe's heel and how much of the texture the coordinates use. Add levels of detail makes the Medium and Low levels from the High one, with the shares you set; Repair rig and Clean weights mend the skin; Mirror model turns left and right over (it needs the game folder, for the ped's skeleton); Crop texture coordinates moves the coordinates into the part of the texture they use and cuts the textures to it. Each makes a new source the garment names, leaves the old one where it is, and Undo takes it back; the box under the buttons draws the model as it was before the last step so you can see the difference.
- Undo clothes change takes back the newest change to the pack. The History panel of the Project workspace lists the steps of the project itself, and a click on one goes back to it.

## Check it on the ped

The Wardrobe dresses the ped in the garment you selected. Drag to orbit, use the wheel to zoom, and use Frame under the view to bring the whole ped back. The tools stand in four rows that keep their places: who wears it (Male and Female, Skin, Outfit and Heels), which version you see (Without the pack, With the pack, or Before the last change), how it is drawn (Draw, Level and Tint), and the ped's own parts, the motion and the pictures. Ped without hair takes the ped's own hair off for the view only, so you can see a hat on a bare head. Bones, Grid, First person and Light stand under the view, and the controls of a clip that plays stand under it too, so that the legs and the shoes stay in sight. Draw says in a line under each choice what it draws, and offers Shaded once the game's shaders are baked. With no game install chosen the garment is shown alone: the view says so, with the button that chooses one, and Skin and Without the pack wait for it. Drop an image on a texture tile and the garment names a new textures source made from it; the old texture stays where it was, and Undo names it again. To work on the image in your editor, save it in that source's `.textures.src` folder and the texture is cooked again as you save; there is no convert button.

Alternates, which appears when the pack has a set of alternate variations, shows the sets as the game applies them: a hat that changes the hair changes it on the ped, and Model, beside it on a row of its own, says which set drives the garment you selected; you can hold every set back, or wear one as if its garment were on. A ped raised by heels stands on a floor at the height they raise it to. First person puts the camera at the ped's head, as far as the install gives the skeleton, with the engine's default field of view; the view says so, drag turns your head, and Escape or Leave (Esc) brings you back. A garment flagged hidden in the first person view is left out there and a garment wears its first person alternate. Light shows the light the view is drawn by and no other: the studio light of the plain views, where it comes from and how strong it is, or while Draw is Shaded the game daylight: Time of day, which is the viewport's own hour, and Shadows.

## Build

Press `Ctrl+B`, or Project ▸ Build. The Build tab says what the build will write, then writes it to a fresh folder, reads every file back and swaps it in. The resource is the folder named after your build, inside `build/` beside the project file.

Tick Verify with a second build on the Build tab and the build is made a second time into a scratch folder and compared byte for byte; the tab says whether it ran, and it is off to begin with because it cooks every source again.

A build tells you that every file it wrote reloads through OpenRage's readers. It never says that the game loads the pack, because nobody can run the game from here. Where a build wrote a kind of file that somebody can look for in a running game, such as a map, the Build tab says how many of those kinds nobody has recorded seeing the game load, and How to check it in the game under that sentence opens the steps, which are also under Help ▸ In-game acceptance guide.

## Many garments, and places that stay put

- A collection holds 128 drawables of a slot, as the other clothing tools do. Past that, a build writes the slot as the next collection, named like the first with `_1` after it and then `_2`, with files and metadata of its own, and the Problems list says so as a warning. Raise the limit of `slot-cap` in the Rules… window of the Problems list (up to 255) to keep one collection. The heading of a slot in the list says how many collections it will be.
- After a build goes through, OpenRage writes `wardrobe.lock.json` beside `wardrobe.json`, which holds the drawable of every garment as it shipped. Saved outfits, shop tables and scripts name a drawable by its number, so a garment that would take another number stops the next build with the error `renumbered`. Fix… pins the garment back where it was; Waive… says why it may move; `openrage-cli wardrobe freeze --thaw` lets go of the lock. Keep the lock in Git.
- An empty place (a dummy) holds a number and ships no model. In the Rules… window, a garment you add can take the first empty place, and a garment you take out can leave an empty place behind so that the ones after it keep their numbers. The menu of a slot's heading fills the gaps in its numbering with empty places, or moves its empty places to the end.
- A texture can be empty too. In the Textures of a garment, Add an empty texture adds a letter with no file, which the game counts and the textures after it keep their letters; Fill on its row puts a picture or a texture dictionary in it; Move the empty textures to the end, which waits until an empty one stands before one that has a file, puts the empty ones after the textures that have a file. With the Rules… option that lets a garment take an empty place, a texture you add takes the first empty texture the garment holds. Each is a step of Undo.
- A pack can replace the game's own files instead of adding a collection: in the Rules… window, under Replace the game's files, say for each ped which collection of the game, or the base game, its files replace, and pin each garment at the number it replaces. A build writes the files under the game's names and none of the metadata, and says it is not verified in game: nobody has seen the game load a replacement.

## Install on a server

OpenRage does not start or change your server. To use a build:

- Copy the resource folder from `build/` into the `resources` folder of your server.
- Add `ensure` and the resource name to `server.cfg`, then restart the resource.
- Try it in the game. Whether the clothes load is for a run on your server to tell you.
- A resource from somewhere else can be checked before you copy it. `openrage-cli project check-resource` and the folder reads it as a server will: its manifest, the files the manifest names and everything under `stream`. It lists what is wrong, such as a file the manifest names that is not there, the same file name twice or a file that does not open, and exits 1 when something is an error. It cannot tell you whether the game loads a file.
- A whole server can be checked for two resources that ship the same file. `openrage-cli project check-server` and the server's `resources` folder lists each file name two resources stream, as a warning when the two copies differ, each archetype two resources declare and each resource name two folders have; add `--cfg` and your `server.cfg` and it says which of the two starts last. `openrage-cli project check-build`, your project and `--server` with that folder says the same of what your build is about to write, before it is written.

## Look at a game file

Open file… on the start page, or Open folder… on an install, shows a texture, a model or a whole archive without making a project, and writes nothing. To edit a game file and keep what you changed, open or make a project and use Project ▸ Import game files…: the file becomes a source folder of the project (a .ytd a textures source, a .ydr a drawable, a .ydd a drawables, a .yft a fragment, and a .ymt, .ymap or .ytyp a metadata source; any other file is kept as it is), and the game's own file is never touched. The import ends with a toast that says what it made and has an Undo import button, and its report lists each file.

## Ship files as a resource

New project ▸ Files starts a project that copies scripts, interface and configuration as they are into a FiveM resource. Add the files with Project ▸ Add files… (the button of an empty project is the same command), then build as above.

## Share a project

A project is a folder: the project file, your sources, your clothing pack and your files. A zip of the folder or a push to Git sends all of it, so look before you send.

- Project ▸ Check before sharing… lists what to take out. It looks at three things: paths in the project file that only your machine has (a linked folder on another drive, for one), sources taken from the game, which are the game's own files and not yours to hand out, with the manifests of the game a Map content keeps in its `manifests` folder (`openrage-cli maps keep-manifest`), which are the game's files whole, and the `.openrage` folder, which holds this project's history and the files only your machine can name.
- A new project starts with a `.gitignore` and a `.gitattributes`. OpenRage manages only the lines between its two marker lines, and every other line stays yours. The block leaves out `build/`, `vfs/`, the sources taken from the game and the manifests of the game a Map content keeps, and marks textures and models as binary. A folder you adopt that already has either file is left as it is, until an import writes the block.
- Select nothing in Assets to see the project's own Properties: In a repository chooses what a repository holds, the sources (to begin with), the build, or the sources taken from the game. The choice is written into the managed block, and an import keeps it.
- A project open in another OpenRage window is read here and written there: this window says so, and changes nothing of the project. It stays that way when the other window lets go: open the project again here to write it.
- The check is something you run. OpenRage installs no Git hook and cannot see a push. `openrage-cli project check-sharing` makes the same check with no window and writes nothing. It exits 1 when the project file holds a path of your machine, a source is taken from the game or a Map content keeps a manifest of the game, so a pre-push hook of your own can run it; it lists the `.openrage` folders too, which Git never sees, and they do not make it exit 1; `openrage-cli project check-original` compares each source taken from the game with the game's file it was taken from.
- A tooltip in the Wardrobe that ends with a line beginning Shift+F1 names the section of this guide that explains the control: press Shift+F1 with the pointer on the control to open it there.
- Project ▸ Save as template… shares the content and builds of a project with no file and no path of your machine in it.

## Keys

- `Ctrl+B` builds.
- `Ctrl+1` to `Ctrl+9` go to the tab in that place of the tab row: the screens of the project first, then the files.
- Help ▸ Keyboard shortcuts… lists the rest and lets you change them.

The app is in English only for now.
