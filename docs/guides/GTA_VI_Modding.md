# GTA VI modding guide

What you can do with a GTA VI install in OpenRage today, in the order you would do it: give it your executable's keys, find files, open and extract them, edit one and save it as a file of its own, and write a copy of an archive. [File_Support.md](../features/File_Support.md#gta-vi) says which kinds of file read and write.

⚠ An archive OpenRage writes is one of two kinds, and neither has been run in the game: a rebuilt archive, whose table changed, is refused by the game because only the game's maker can sign a table, and a patched copy keeps the signed table byte for byte, so no check read in the game's code rejects it, but no run of the game was made with one ([the limit](#the-limit)).

## Give OpenRage your executable's keys

An archive's table of contents is enciphered, and the keys that decipher it are in the game's executable. OpenRage ships no key: it reads them out of your own executable.

- `openrage-cli keys gta6 --exe <executable>` reads the keys from the executable of a build OpenRage knows, and refuses any other. It writes them as `gtavi_magic.dat` in OpenRage's configuration folder, masked so that the file opens only with that same executable, and it will not replace a file already there unless you pass `--force` or name another file with `--out`.
- The app unmasks the snapshot with the executable in the GTA VI folder set in Settings ▸ Games, and the CLI with the one `--exe <executable>` names; the environment variable `OPENRAGE_GTA6_EXE` names it for both. A masked snapshot is refused when no executable is given or it is another one.
- With an executable given and no snapshot file found, the masked snapshot OpenRage carries opens with it, or the keys of a build OpenRage knows are read out of it, and either is kept masked in the configuration folder, so the first step can be skipped.
- `OPENRAGE_GTA6_KEYS` names a snapshot file elsewhere, and every `openrage-cli rpf9` verb takes `--keys <file>` for one run, with `--exe <executable>` for a masked one.

The snapshot holds three kinds of key: the table key, the content key that deciphers an enciphered entry, and the string key that deciphers a metadata document's strings. A step below that needs one says so when it is missing.

## Find files

With the keys, a GTA VI install is browsed exactly as a GTA V one is: one Files tree, in which an archive is a folder, a nested archive is a folder inside it, and a double-click opens a file. There is no separate archive panel.

- **In the app.** Open the install folder. Each archive opens in the Files tree from its deciphered table: a folder for each folder its names make, an entry no name resolves under its hash, and a nested archive as a folder of its own. The filter finds a row by its name, its kind or its hash in the `0x` form Copy hash gives. The Inspector shows a selected entry's kind, its stored and real sizes, whether it is enciphered, the shape of its row and, where no name is held, its name hash.
- **In the CLI.** `openrage-cli rpf9 list <archive> [inner/path]` prints an archive's entries, with `--nested` one level of nested archives too, `--groups` the table's columns and `--json` one JSON document. The inner path walks into nested archives, and a component or an `--entry` names an entry by its name, by its hash as `0x…` or `hash_…`, or by its number. `openrage-cli rpf9 find <folder>` finds entries across every archive of an install, nested ones included, by `--name`, `--hash` or `--kind`, and `openrage-cli rpf9 census <folder> --out <folder>` writes the counts of entries, nested archives and enciphered entries per archive, never an entry's name, as a Markdown table.
- **With no key, a scan.** Without the keys, or where a table does not open with the keys held, an archive is listed by a scan of the bodies it stores in the clear, in a tab of its own, and a message says why. Only then does the install folder's view show a bar that says why no table opened, with Scan every archive, which tallies each archive's bodies by kind and finds one by archive, offset or kind, and Harvest names. A scan finds no enciphered body, and it addresses a body by its offset, not its name.

### Names

A row names its entry only by the 64-bit hash of its path. OpenRage shows a name where it holds a text that hashes to it, and the hash otherwise. No list of the game's paths ships with OpenRage. The names come from these places, and where two give one hash a name, your own names win:

- **Your names folder**: the one `--names <folder>` names for a CLI run, or otherwise `OPENRAGE_GTA6_NAMES`, for the app and the CLI. It may hold `resolved-names.tsv`, whose rows are `0xHASH<TAB>name`, the tables of a recovered-names folder, and `vocabulary.txt`, one name a line.
- **What you mined and harvested**, kept in the `gta6_names` folder of the configuration folder. `openrage-cli hashes mine <install> --out <file>` tries the strings the install's bodies state, and with `--exe` those of your executable, against the hashes no name resolves, and keeps a candidate only where it hashes to one. It writes what it finds to the file `--out` names as `0xHASH<TAB>name` rows, keeping the rows already in it, and merges every 64-bit name it found into `resolved-names.tsv` in that `gta6_names` folder, which the tree reads, so its names show without `--out` naming a file there. The same folder holds the vocabulary that Harvest names, the Harvest GTA VI names command and `openrage-cli rpf scan-names` keep from the names the bodies spell. Listing or opening an archive never writes either, and none of it is written into the repository.
- **The cache.** Your names folder and that `gta6_names` folder are read through one cache file in the configuration folder: `gtavi_names_cache.dat` with no names folder, and with one a file beside it named by a digest of the files read, so another names folder keeps a cache of its own. A cache is keyed by the path, size and modification time of every file it was read from, so a run after the first reads that one file while its sources stand, and a source that changed, or a cache copied to another configuration folder, rebuilds it.
- **In the app, the names OpenRage holds with no game data**: those of the installed schema snapshot, the public names and the parser names, each under its path hash.
- **The bundled words**, last: plain words shared across titles and hash functions, in the data files of `or-io`, with no hash column, no build-specific path and no game file extension.

The app builds its names once a run, on a worker, before an archive asks for them. Missing names still show as hashes.

## Open and extract

- **In the app.** In the Files tree, a double-click on an entry opens it, and a single click opens it in the preview tab. A body stored in the clear opens in the view its kind has. An enciphered body is deciphered on a worker with the content key: a PSO or RBF opens in the metadata editor, text as text, a resource in the view a clear one of its kind shows, and anything else as its bytes. Extract writes a row's body as the open reads it.
- **A metadata document's strings.** A PSO document's enciphered strings are deciphered with the string key and name the hash values the document holds; without it the document opens and says that the names need the key snapshot.
- **In the CLI.** `openrage-cli rpf9 extract <archive> <folder> [inner/path]` writes each entry's body, inflated, under its name or its hash, or only the one `--entry` names or those whose name holds `--filter <text>`. An enciphered entry is written only with `--decrypt`, deciphered first; a nested archive is written as the bytes its row states, an archive that opens on its own.
- **By scanning, with no key.** `openrage-cli rpf extract` writes the bodies a scan finds, or those of one family with `--kind`, and with `--as converted` writes textures as PNG, models as glTF, metadata as XML and audio as Ogg Opus; `openrage-cli rpf scan-body` writes one body, as it is stored with `--stored`, a metadata body's XML with `--xml` or a text table as GTA V's text table with `--gxt2`.
- **Scripts.** `openrage-cli script compiled <body>` reads a deciphered script body, which `rpf9 extract --decrypt` writes, as a compiled program: its header, its instructions and its native calls, with `--listing` every instruction at its code offset, and with `--natives` each call's native by its address in the executable, never by a name. `openrage-cli script decompile <body>` writes the same body as source, and `script export` every program of an archive or a folder, with names only from the lists you supply ([GTA_VI_Script_Decompiler.md](../features/GTA_VI_Script_Decompiler.md)). [A script's values](#a-scripts-values) says how one is edited.

## Edit and save a file of your own

A body opened from an archive is never written into the archive it was read from, and a save over that archive is refused.

- **Save As** writes the body, with the edits its view takes, as a file of its own. A name ending in `.xml` writes a metadata body's XML and one ending in `.gxt2` a text table as GTA V's text table; a resource saves as a loose resource, which OpenRage opens again on its own. A view holding edits that no writer takes refuses Save As rather than drop them.
- **Checksums.** A GTA VI `zppp` PSO's `CHKS` is computed again on save. A frame whose rule OpenRage does not reproduce keeps the checksum it was read with, and the save of an edited one warns that the game may refuse the file for it.
- **In the CLI.** `openrage-cli rpf scan-from-xml` writes an edited XML back into a headerless metadata body as a loose resource, and `openrage-cli ttpf replace` writes a texture pack with one texture replaced from an image, to the file `-o` names.

Which values of a model, a bound, a map or a metadata body can be edited is in [File_Support.md](../features/File_Support.md#gta-vi), and a refusal names what it cannot write.

## Write a copy of an archive

Every write goes to a new file: the source archive is only read, the file named for the copy must not exist yet, and a write that fails leaves no partial file behind. Each write says which of the two kinds it wrote.

- **A patched copy** puts each changed body in its entry's place, chunk by chunk, and keeps every header, table and signature byte as the source holds it. The table's signature still matches and no check read in the game's code rejects such a copy, but no run of the game was made with one, so whether the game loads it is not known.
- **A rebuilt archive** has rows added, removed, renamed or resized, or its table laid out again. The game checks a signature over the table when it mounts an archive and only its maker can sign one, so the game refuses a rebuilt archive.

A body goes into a patched copy only at its row's real size, and each changed chunk only where its new DEFLATE stream fits the stored size the signed table gives that chunk. So the edit has to keep the body's length: a value changed in place, not a text lengthened or a row added.

### One value, from the Files tree to a patched copy

The path below changes one value of a metadata body and saves the archive as a new file, in the app; [the same in the CLI](#a-patched-copy-step-by-step) follows it.

1. **Find the row.** With the keys held, the install folder's Files tree shows each archive as a folder and a nested archive as a folder inside it; `openrage-cli rpf9 find` finds the archive that holds a name too. Open that archive on its own with File ▸ Open file…, since a GTA VI archive is saved only from a tree of its own. Type part of the row's name, its kind or its hash in the `0x` form Copy hash gives into the filter, and select the row: the Inspector shows its kind, its stored and real sizes and whether it is enciphered.
2. **Open it.** Double-click the row. A PSO or RBF body opens in the metadata editor, deciphered on a worker first where its row is enciphered.
3. **Edit it.** Change values in place, such as a number or a flag. A text made longer, an item added or an item removed changes the body's length, and the archive then saves as a rebuilt archive.
4. **Save it into its row.** Save writes the document's edits into the row it was read from, in the tree only: the row shows as edited, and no file is written yet. Where a PSO's checksum is kept as it was read, a warning says that the game may refuse the file for it.
5. **Save the archive.** File ▸ Save as… of the tree plans the save on a worker, then says which copy it writes before asking where: "The edits to", the archive's name and "save as a patched copy" when every changed body fits its entry's chunks, or "save as a rebuilt archive, which the game refuses because only its maker can sign the table", followed by "These rows keep it from being patched in place:" and each row with its reason. The reasons are a body some bytes over or short of the size its row states, a chunk's new stream some bytes over the stored size its table gives, a headerless resource, whose loose form is stored under a new row, and a row renamed, removed or added. With nothing edited, the copy is the archive as it was read, byte for byte. The dialog then asks for a new file, offering the archive's name with `_edited`; the archive read is never written, and a file already at the name picked is refused.
6. **Read what it wrote.** The message after the save says it again of the file written: "is a patched copy", "is the archive as it was read", or a new archive whose table changed, which the game refuses.

File ▸ Save into a copy of the archive… saves the document's edits into a new copy of the archive it was read from in one go: it asks where first, under the title "Save a copy", and says after the write which copy it is: a patched copy, the archive as it was read, or a rebuilt archive with the rows that kept it from being patched. A model, a bound, a navigation cell and every other headerless resource save from the app as a rebuilt archive; to patch one, give `rpf9 patch` the inflated body `rpf9 extract` writes, as below.

### A patched copy, step by step

This is the CLI's path, and the only one for a body the app saves rebuilt. Below, `<entry>` names an entry of `<archive.rpf>` by its name, its hash or its number, and `<path>` is that same `<entry>`; for an entry of a nested archive, step 1 gives the nested archive's path as `[inner/path]`, and `<path>` is that inner path, a slash and the entry. `openrage-cli rpf9 find <install> --name <text>` prints, for each entry whose name holds the text, its archive, the inner path of the nested archives above it and the entry itself.

1. Extract the body: `openrage-cli rpf9 extract <archive.rpf> <folder> [inner/path] --entry <entry> --decrypt`. It is written inflated and, where its row is enciphered, deciphered.
2. Edit that file without changing its length.
3. Ask whether it fits, which writes nothing: `openrage-cli rpf9 fits <archive.rpf> --entry <path> --with <edited file>`. It prints, for each chunk, whether it is unchanged and keeps its stored bytes, or how many bytes its new stream takes under its ceiling and how many are to spare; a body that does not fit is named with the reason, its length or the chunk that outgrows its ceiling, and the command exits with failure.
4. Write the copy: `openrage-cli rpf9 patch <archive.rpf> --entry <path> --with <edited file> --out <new.rpf>`. It says how many chunks changed and how many bytes it wrote, and that the copy is a patched copy. A row it cannot put in place is refused by name and nothing is written; `rpf9 replace` would rebuild the archive instead, which the game refuses.

`--entry` and `--with` repeat in pairs, one pair a row, and both verbs take `--names`, `--keys` and `--exe` as the other `rpf9` verbs do. An enciphered row is enciphered again where it lies, and a nested archive itself is refused: patch the entries inside it.

### A script's values

A compiled script is edited by the values its pushes push, never by its code, and only in the CLI. What the edit writes is a script body of the source's length, not an archive.

1. Extract the deciphered body: `openrage-cli rpf9 extract <archive.rpf> <folder> [inner/path] --entry <entry> --decrypt`.
2. Find the push to change: `openrage-cli script compiled <body> --listing` prints every instruction by function, at its code offset.
3. Write the edited body: `openrage-cli script compiled <body> --set-int <offset>=<value> --set-float <offset>=<value> --out <new body>`. Each `--set-int` sets the integer the push at that code offset pushes, and each `--set-float` the 32-bit float a 4-byte push pushes; the flags repeat, an offset or an integer may be written in `0x` hex, and every other byte of the body is written as it was read. The new file must not exist yet.
4. Put the body into a patched copy with `rpf9 fits`, then `rpf9 patch`, [as above](#a-patched-copy-step-by-step).

Refused by name, and nothing written: an offset at which no instruction starts, a branch, whose operand is a code position, an instruction that is not a push of one immediate value, an integer the push's immediate cannot hold, and a float for a push that is not 4 bytes. `--set-string` is refused: every string constant of the build's compiled programs names one same object, so no string text was found to replace. No run of the game was made with an edited program, so whether the game runs one is not known.

### What fits today

The patch writer deflates a changed chunk with each of zlib's levels and, where none fits its stored size, with an optimal parse (zopfli), but only on a chunk no larger inflated than `OPTIMAL_CAP` in `crates/io/or-io/src/container/rpf/gta6_patch.rs`, because the parse's time grows with the chunk; a larger chunk's refusal names the cap and says that the optimal parse was not tried on it. That cap is what `rpf9 fits` and `rpf9 patch` use; a save from the app runs the parse only on chunks of up to 256 KiB, since its plan cannot yet be stopped once it starts. A stream shorter than its stored size is filled with empty blocks or followed by zeros, which the reader and the game pass over. What fits:

- a chunk the edit leaves alone, whose stored bytes are kept as they are;
- a chunk the game stored without compression, which takes any content of its exact length;
- a changed chunk whose new content deflates within its stored size at one of zlib's levels, or, within the cap, under the optimal parse, which `rpf9 fits` tells you before anything is written.

The figures below were measured on 2026-11-25 on one Xbox build at `57eece64`, whose cap was 256 KiB, and say what fits that build's signed tables, not what the game loads:

- **Unchanged chunks deflated again.** Over a sample of at most 40 deflated chunks of 256 KiB or less per kind, in bodies of at most 4 MiB, `gta6_edits_that_fit_in_place` fitted 350 of 358 chunks within their stored size with the optimal parse and 5 of the same 358 with zlib's best level. The game's compressor packs tighter than zlib: on a wider sample of 1,613 bodies of ten kinds, read by a script outside the tree at zlib's level 9, 42 of 3,249 deflated chunks fitted, and zlib's stream was between about 1 % and 11 % larger than the game's, by kind. The suite asserts that zlib fits fewer than one deflated chunk in twenty and prints the optimal parse's counts.
- **A one-byte edit.** `gta6_patch_in_place` changes one byte of sampled rows of each row shape and patches it in: 14 of 17 rows fitted. Two of the three refused missed their chunk's stored size by one byte under the optimal parse, and the third, an enciphered row, has a chunk past that 256 KiB cap, on which the optimal parse was not tried.
- **What a cap leaves out.** Counted on 2026-11-26 at `57eece64` by a script outside the tree from the chunk tables of every archive of the Xbox build 1.0.2.1, nested ones included, without inflating a body: 208,477 of the 2,243,921 deflated chunks (9.3 %) are larger than 256 KiB, and 205,452 of the 1,178,052 rows with a chunk table (17.4 %) hold at least one such chunk; past 1 MiB lie 74,667 of the chunks (3.3 %), in 74,526 of those rows (6.3 %); past 8 MiB 1,886 (0.08 %), in 1,885 of them (0.16 %); and no chunk is larger than 32 MiB. A changed chunk past the cap fits only where a zlib level fits it; zlib fitted about one sampled chunk in a hundred above, and those samples held no chunk past the cap.

Both suites are install-gated: they skip without the keys and the build that `OPENRAGE_GTA6_KEYS` and `OPENRAGE_GTA6_BUILD` name, and a skip is not a pass.

### A rebuilt archive

The other verbs lay a table out again, and each that rewrote a table says that its signature is stale and that the game refuses the archive.

- `openrage-cli rpf9 copy <archive> --out <new>` deciphers each table, enciphers it again and copies every body; a copy that changed nothing keeps every table and signature as it read them.
- `rpf9 replace` sets an entry's body from the file `--with` names, `rpf9 remove` drops an entry and `rpf9 rename` gives one the path `--name` names; each takes the inner path and the `--entry` selector `rpf9 list` takes, and an entry of a nested archive is written in the nested archive first, then in its parent's row. `rpf9 replace` names `rpf9 patch` where its change would have fitted in place.
- `rpf9 defrag` packs the bodies in order, `rpf9 add` adds an entry and `rpf9 build` makes an archive from a folder.

A replaced body is stored in the clear, so it loses its content cipher, and one that OpenRage's reader would not read back is refused. Refused by name too are a plain replace of an entry whose row describes a paged resource, which `rpf9 replace --resource` replaces by its loose form instead, a replace of a nested archive itself rather than of the entries inside it, and an edit of a table that, laid out again unedited, would not come out as it was read.

## The limit

The game verifies a signature over an archive's table of contents when it mounts the archive, and only the game's maker can sign one, so the game refuses every rebuilt archive: those of `rpf9 replace`, `remove`, `rename`, `defrag`, `add` and `build`, of `dlc create-gta6`, and of the app's saves when a change does not fit. A patched copy keeps the signed table byte for byte, and no check read in the game's code rejects it; no run of the game was made with one. OpenRage checks structure, that a written archive reopens, lists and reads back, and claims nothing about the game loading it.
