# GTA VI modding guide

What you can do with a GTA VI install in OpenRage today, in the order you would do it: give it your executable's keys, find files, open and extract them, edit one and save it as a file of its own, and write a copy of an archive. [File_Support.md](../features/File_Support.md#gta-vi) says which kinds of file read and write.

⚠ An archive OpenRage writes is a structural result, not a mod the game loads: the game refuses every one whose table changed, because only the game's maker can sign a table ([the limit](#the-limit)), and no file OpenRage saved has been tried in the game.

## Give OpenRage your executable's keys

An archive's table of contents is enciphered, and the keys that decipher it are in the game's executable. OpenRage ships no key: it reads them out of your own executable.

- `openrage-cli keys gta6 --exe <executable>` reads the keys from the executable of a build OpenRage knows, and refuses any other. It writes them as `gtavi_magic.dat` in OpenRage's configuration folder, masked so that the file opens only with that same executable, and it will not replace a file already there unless you pass `--force` or name another file with `--out`.
- The app unmasks the snapshot with the executable in the GTA VI folder set in Settings ▸ Games, and the CLI with the one `--exe <executable>` names; the environment variable `OPENRAGE_GTA6_EXE` names it for both. A masked snapshot is refused when no executable is given or it is another one.
- With an executable given and no snapshot file found, the masked snapshot OpenRage carries opens with it, or the keys of a build OpenRage knows are read out of it, and either is kept masked in the configuration folder, so the first step can be skipped.
- `OPENRAGE_GTA6_KEYS` names a snapshot file elsewhere, and every `openrage-cli rpf9` verb takes `--keys <file>` for one run, with `--exe <executable>` for a masked one.

The snapshot holds three kinds of key: the table key, the content key that deciphers an enciphered entry, and the string key that deciphers a metadata document's strings. A step below that needs one says so when it is missing.

Without a snapshot the tables stay closed, and OpenRage still finds the bodies an archive stores in the clear by scanning it, which the next section uses.

## Find files

An entry is named by a hash in the table. OpenRage shows a name where it knows the text that hashes to it, and the hash otherwise.

- **Names.** The app and CLI share bundled word candidates, the local cache and vocabulary, and optional external name tables. The bundled candidates are plain words shared across titles and hash functions, grouped in the crate's data files; no hash column, build-specific paths or game file extensions ship in those lists. They are lexical words split from names harvested in the user's GTA V and GTA VI files, with common asset terms added. The complete recovered paths stay local.
  The cache is `gta6_names/resolved-names.tsv` under the configuration folder, with `0xHASH<TAB>name` rows. Only `hashes mine` writes it: it merges verified 64-bit matches into it atomically without discarding earlier names, and listing or opening an archive never does. The Harvest GTA VI names command and the Harvest names button keep the body vocabulary in that same local folder. A folder passed through `--names <folder>`, or otherwise `OPENRAGE_GTA6_NAMES`, takes priority over the local cache; the local names take priority over the bundled candidates. Missing names still show as hashes.
- **In the app, by scanning.** Open the install folder. Above its Files view, Scan every archive tallies each archive's bodies by kind, and its find field finds a body by archive, offset or kind, or goes to an offset. This needs no key, and it finds only the bodies stored in the clear.
- **In the app, by the table.** View ▸ Panels ▸ Archives lists the archives of the GTA VI folder set in Settings, or of the folder `OPENRAGE_GTA6_BUILD` names in its place, and each archive File ▸ Open opened. A click on one deciphers its table and lists every entry with its name or hash, its kind and its size, an enciphered one marked `[cipher]`; a nested archive expands in place. Its search keeps the entries whose name holds the text, whose hash it gives as `0x` hex, or whose kind is toggled on, among the archives expanded.
- **In the CLI.** `openrage-cli rpf9 list <archive> [inner/path]` prints an archive's entries, with `--nested` one level of nested archives too, `--groups` the table's columns and `--json` one JSON document. The inner path walks into nested archives, and a component or an `--entry` names an entry by its name, by its hash as `0x…` or `hash_…`, or by its number. `openrage-cli rpf9 census <folder> --out <folder>` counts the entries, nested archives and enciphered entries of an install's archives, and writes the counts per archive, never an entry's name, as a Markdown table into the folder `--out` names.

## Open and extract

- **In the app.** In the Archives panel, a double-click on an entry opens it. A body stored in the clear opens in the view its kind has. An enciphered body is deciphered on a worker with the content key: a PSO or RBF opens in the metadata editor, text as text, a resource in the view a clear one of its kind shows, and anything else as its bytes. A double-click on a nested archive opens it in a tab of its own, read by scanning.
- **A metadata document's strings.** A PSO document's enciphered strings are deciphered with the string key and name the hash values the document holds; without it the document opens and says that the names need the key snapshot.
- **In the CLI, from the table.** `openrage-cli rpf9 extract <archive> <folder> [inner/path]` writes each entry's body, inflated, under its name or its hash, or only the one `--entry` names or those whose name holds `--filter <text>`. An enciphered entry is written only with `--decrypt`, deciphered first; a nested archive is written as the bytes its row states, an archive that opens on its own. `openrage-cli rpf9 find <folder>` finds entries across every archive of an install, nested ones included, by `--name`, `--hash` or `--kind`.
- **In the CLI, by scanning.** `openrage-cli rpf extract` writes the bodies a scan finds, or those of one family with `--kind`, and with `--as converted` writes textures as PNG, models as glTF, metadata as XML and audio as Ogg Opus; `openrage-cli rpf scan-body` writes one body, as it is stored with `--stored`, a metadata body's XML with `--xml` or a text table as GTA V's text table with `--gxt2`.
- **Scripts.** `openrage-cli script compiled <body>` reads a deciphered script body, which `rpf9 extract --decrypt` writes, as a compiled program: its header, its instructions and its native calls, and with `--natives` each call's native by its address in the executable, never by a name.

## Edit and save a file of your own

A body opened from an archive is read-only there: OpenRage never writes into the archive it read from, and a save over it is refused.

- **Save As** writes the body, with the edits its view takes, as a file of its own. A name ending in `.xml` writes a metadata body's XML and one ending in `.gxt2` a text table as GTA V's text table; a resource saves as a loose resource, which OpenRage opens again on its own. A view holding edits that no writer takes refuses Save As rather than drop them.
- **Checksums.** A GTA VI `zppp` PSO's `CHKS` is computed again on save. A frame whose rule OpenRage does not reproduce keeps the checksum it was read with, and the save of an edited one warns that the game may refuse the file for it.
- **In the CLI.** `openrage-cli rpf scan-from-xml` writes an edited XML back into a headerless metadata body as a loose resource, and `openrage-cli ttpf replace` writes a texture pack with one texture replaced from an image, to the file `-o` names.

Which values of a model, a bound, a map or a metadata body can be edited is in [File_Support.md](../features/File_Support.md#gta-vi), and a refusal names what it cannot write.

## Save into a copy of an archive

The CLI writes a new archive from an existing one, and so does the app: File ▸ Save into a copy of the archive… writes the edits of a document opened from an archive in place of its entry, in a new copy of that archive.

- `openrage-cli rpf9 copy <archive> --out <new>` deciphers each table, enciphers it again and copies every body.
- `rpf9 replace` sets an entry's body from the file `--with` names, `rpf9 remove` drops an entry and `rpf9 rename` gives one the path `--name` names; each takes the inner path and the `--entry` selector `rpf9 list` takes, and an entry of a nested archive is written in the nested archive first, then in its parent's row.
- `rpf9 defrag` packs the bodies in order, so only their starts and the table change.

The source archive is only read. `--out` must name a file that does not exist yet, and the source itself is refused; a write that fails leaves no partial file behind. A replaced body is stored in the clear, so it loses its content cipher, and one that OpenRage's reader would not read back is refused. Refused by name too are a plain replace of an entry whose row describes a paged resource, which `rpf9 replace --resource` replaces by its loose form instead, a replace of a nested archive itself rather than of the entries inside it, and an edit of a table that, laid out again unedited, would not come out as it was read.

Each write says what it changed. A copy that changed nothing keeps every table and signature as it read them; one that rewrote a table says that the table's signature is stale and that the game refuses the archive.

## The limit

The game verifies a signature over an archive's table of contents when it mounts the archive, and only the game's maker can sign one. An edited table cannot carry a valid signature, so the game refuses every archive OpenRage writes in which a table changed. OpenRage checks structure, that a written archive reopens, lists and reads back, and claims nothing about the game loading it.
