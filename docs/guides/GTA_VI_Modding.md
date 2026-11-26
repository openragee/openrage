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
- **What you mined and harvested**, kept in the `gta6_names` folder of the configuration folder: the `resolved-names.tsv` that `openrage-cli hashes mine <install> --out <file>` writes when `--out` names it there, keeping the rows already in it, and the vocabulary that Harvest names, the Harvest GTA VI names command and `openrage-cli rpf scan-names` keep from the names the bodies spell. Listing or opening an archive never writes either.
- **The cache.** Those two are read through one file, `gtavi_names_cache.dat` in the configuration folder, keyed by the size and modification time of every file they were read from, so a run after the first reads that one file while its sources stand, and a source that changed rebuilds it.
- **In the app, the names OpenRage holds with no game data**: those of the installed schema snapshot, the public names and the parser names, each under its path hash.
- **The bundled words**, last: plain words shared across titles and hash functions, in the data files of `or-io`, with no hash column, no build-specific path and no game file extension.

The app builds its names once a run, on a worker, before an archive asks for them. Missing names still show as hashes.

## Open and extract

- **In the app.** In the Files tree, a double-click on an entry opens it, and a single click opens it in the preview tab. A body stored in the clear opens in the view its kind has. An enciphered body is deciphered on a worker with the content key: a PSO or RBF opens in the metadata editor, text as text, a resource in the view a clear one of its kind shows, and anything else as its bytes. Extract writes a row's body as the open reads it.
- **A metadata document's strings.** A PSO document's enciphered strings are deciphered with the string key and name the hash values the document holds; without it the document opens and says that the names need the key snapshot.
- **In the CLI.** `openrage-cli rpf9 extract <archive> <folder> [inner/path]` writes each entry's body, inflated, under its name or its hash, or only the one `--entry` names or those whose name holds `--filter <text>`. An enciphered entry is written only with `--decrypt`, deciphered first; a nested archive is written as the bytes its row states, an archive that opens on its own.
- **By scanning, with no key.** `openrage-cli rpf extract` writes the bodies a scan finds, or those of one family with `--kind`, and with `--as converted` writes textures as PNG, models as glTF, metadata as XML and audio as Ogg Opus; `openrage-cli rpf scan-body` writes one body, as it is stored with `--stored`, a metadata body's XML with `--xml` or a text table as GTA V's text table with `--gxt2`.
- **Scripts.** `openrage-cli script compiled <body>` reads a deciphered script body, which `rpf9 extract --decrypt` writes, as a compiled program: its header, its instructions and its native calls, and with `--natives` each call's native by its address in the executable, never by a name.

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

### A patched copy, step by step

A body goes into a patched copy only at its row's real size, and each changed chunk only where its new DEFLATE stream fits the stored size the signed table gives that chunk. So the edit has to keep the body's length: a value changed in place, not a text lengthened or a row added. Below, `<entry>` names an entry of `<archive.rpf>` by its name, its hash or its number, and `<path>` is that same `<entry>`; for an entry of a nested archive, step 1 gives the nested archive's path as `[inner/path]`, and `<path>` is that inner path, a slash and the entry.

1. Extract the body: `openrage-cli rpf9 extract <archive.rpf> <folder> [inner/path] --entry <entry> --decrypt`. It is written inflated and, where its row is enciphered, deciphered.
2. Edit that file without changing its length.
3. Ask whether it fits, which writes nothing: `openrage-cli rpf9 fits <archive.rpf> --entry <path> --with <edited file>`. It prints, for each chunk, whether it is unchanged and keeps its stored bytes, or how many bytes its new stream takes under its ceiling and how many are to spare; a body that does not fit is named with the reason, its length or the chunk that outgrows its ceiling, and the command exits with failure.
4. Write the copy: `openrage-cli rpf9 patch <archive.rpf> --entry <path> --with <edited file> --out <new.rpf>`. It says how many chunks changed and how many bytes it wrote, and that the copy is a patched copy. A row it cannot put in place is refused by name and nothing is written.

`--entry` and `--with` repeat in pairs, one pair a row, and both verbs take `--names`, `--keys` and `--exe` as the other `rpf9` verbs do. An enciphered row is enciphered again where it lies, and a nested archive itself is refused: patch the entries inside it.

In the app, Save as… of an archive's tree and File ▸ Save into a copy of the archive… of a document opened from one write a patched copy when every changed row is a body of its entry's size that fits its chunks, and otherwise a rebuilt archive, whose message names each row that kept the copy from being patched and why.

### What fits today

The patch writer deflates a changed chunk with each of zlib's levels and, where none fits its stored size, with an optimal parse (zopfli) on a chunk of at most 256 KiB; a stream shorter than its stored size is filled with empty blocks or followed by zeros, which the reader and the game pass over. The game's compressor packs its chunks tighter than zlib's best level, so few chunks fit zlib's streams. That was measured on 2026-11-25 over one Xbox build, by a script outside the tree, on a sample of 1,613 bodies of ten kinds: deflated again unchanged at zlib's level 9, 42 of 3,249 deflated chunks fitted their stored size and 5 bodies fitted whole, and zlib's stream was between about 1 % and 11 % larger than the game's, by kind. `crates/io/or-io-verify/tests/gta6_edits_that_fit_in_place.rs` asserts on a build that fewer than one deflated chunk in twenty fits, and skips without the keys and the build that `OPENRAGE_GTA6_KEYS` and `OPENRAGE_GTA6_BUILD` name. What does fit:

- a chunk the edit leaves alone, whose stored bytes are kept as they are;
- a chunk the game stored without compression, which takes any content of its exact length;
- a changed chunk whose new content deflates within its stored size at one of zlib's levels, which `rpf9 fits` tells you before anything is written;
- a changed chunk the optimal parse fits where no zlib level does.

### A rebuilt archive

The other verbs lay a table out again, and each that rewrote a table says that its signature is stale and that the game refuses the archive.

- `openrage-cli rpf9 copy <archive> --out <new>` deciphers each table, enciphers it again and copies every body; a copy that changed nothing keeps every table and signature as it read them.
- `rpf9 replace` sets an entry's body from the file `--with` names, `rpf9 remove` drops an entry and `rpf9 rename` gives one the path `--name` names; each takes the inner path and the `--entry` selector `rpf9 list` takes, and an entry of a nested archive is written in the nested archive first, then in its parent's row. `rpf9 replace` names `rpf9 patch` where its change would have fitted in place.
- `rpf9 defrag` packs the bodies in order, `rpf9 add` adds an entry and `rpf9 build` makes an archive from a folder.

A replaced body is stored in the clear, so it loses its content cipher, and one that OpenRage's reader would not read back is refused. Refused by name too are a plain replace of an entry whose row describes a paged resource, which `rpf9 replace --resource` replaces by its loose form instead, a replace of a nested archive itself rather than of the entries inside it, and an edit of a table that, laid out again unedited, would not come out as it was read.

## The limit

The game verifies a signature over an archive's table of contents when it mounts the archive, and only the game's maker can sign one, so the game refuses every rebuilt archive: those of `rpf9 replace`, `remove`, `rename`, `defrag`, `add` and `build`, of `dlc create-gta6`, and of the app's saves when a change does not fit. A patched copy keeps the signed table byte for byte, and no check read in the game's code rejects it; no run of the game was made with one. OpenRage checks structure, that a written archive reopens, lists and reads back, and claims nothing about the game loading it.
