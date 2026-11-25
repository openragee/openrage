# CLI server protocol

The wire contract of `openrage-cli serve`, a long-lived process that mounts a
GTA V install once and answers requests over stdin and stdout, for programs
that embed it.

It is the external plugin surface ([the decision](../direction/Project_Context.md#a-plugin-is-wasm-in-the-app-and-a-subprocess-outside-it-2026-09-21),
[R43](../direction/Roadmap.md#r43-the-external-surface-and-open-with)).
`crates/tools/or-cli/tests/the_external_surface_is_what_it_documents.rs` holds
it to `crates/tools/or-cli/src/serve.rs`: every route is named here, every
`"cmd"` shown here is answered, and every route reads except the four that change a
clothing pack ([Routes that change a pack](#routes-that-change-a-pack)). No route changes the
install; `extract` and `extract-many` write only to paths the caller names, and the four write
only the pack of the project the caller names.

## Transport and lifecycle

- The child's stdin carries requests and its stdout responses, as
  newline-delimited JSON: one object per line each way, no sockets, no ports.
  A trailing `\r` is dropped and a line of only whitespace gets no response.
- Tracing and key-extraction progress go to stderr, so stdout carries only the
  protocol. Nothing is printed at startup.
- One request at a time, answered in the order received.
- The process exits 0 after answering `shutdown`, at stdin EOF, or when a write
  to stdout fails; the mounted state lives as long as the process.

## Startup and keys

```
openrage-cli serve [--game <dir>] [--keys <file>] [--dlc <X>]
```

The flags (or `--game=<dir>` and so on) are the defaults `mount` and a raw
`search` use when a request omits the field; they never mount anything. An
unknown flag prints the usage to stderr and exits 2.

Keys live in `<config>/openrage/`, `<config>` being the platform configuration
directory (`%APPDATA%` on Windows):

- A `keys` field or `--keys` names a keys cache, used as it is; one that cannot
  be read leaves the mount without retail keys, with a warning.
- Otherwise the title's own cache, `gta5_keys.bin` (Legacy) or
  `gta5_enhanced_keys.bin` (Enhanced), is used while it matches the SHA-1 of
  the install's executable; else keys are extracted from that executable,
  with progress on stderr, and the cache is rewritten. With neither a cache nor
  the executable, the mount has no retail keys and a warning says so.
- Extraction recovers the AES key, the hash table and the AWC key. NG keys come
  from a key snapshot: the user's own `magic.dat` beside the cache, else the one
  OpenRage bundles (D18), which decodes only with the executable's AES key. A
  cache an earlier build wrote without NG keys is extracted again once, when
  the executable is present; until then, and if that fails, its AES keys serve.

⚠ Otherwise a cache is reused while the executable is unchanged, so a
`magic.dat` placed after it was written is not read. Delete the cache file to
extract again.

## Envelope and errors

```
-> {"id": 1, "cmd": "list", "path": "common:/data"}
<- {"id": 1, "ok": false, "error": "not mounted; call `mount` first"}
-> {"id": 2, "cmd": "search", "raw": true, "pattern": "railgun"}
<- {"id": 2, "ok": true, "count": 3, "hits": [...], "truncated": false, "maxHits": 100000, "warnings": []}
```

- `cmd` is required and names the route; every other field is a parameter.
  `id` is optional, any JSON value, and echoed verbatim when present.
- `ok` is always present. On success the route's result fields are merged into
  the envelope; on failure the only other field is `error`, a sentence.
- A line that is JSON but not a request keeps its `id` in the error; one that
  is not JSON gets an error without one. An unknown `cmd` is an error.
- A line over 16 MiB is drained to its newline and answered with an error
  without an `id`, and the stream stays in step.
- A handler panic is answered as
  `{"ok":false,"error":"internal error (handler panicked): ..."}` and the
  server keeps running.

## Routes

### `ping`

No parameters. The result is `pong` and the build's package `version`.

```
-> {"id":1,"cmd":"ping"}
<- {"id":1,"ok":true,"pong":true,"version":"0.1.0"}
```

### `mount`

Resolves keys and mounts the install, replacing any previous mount; a failed
`mount` leaves the previous one in place. Every parameter is optional:

| Field | Meaning | Default |
|---|---|---|
| `game` | install root | `--game`, then the app's selected game in `<config>/openrage/prefs.json` (`active_game` → `game_paths`) |
| `keys` | a keys cache file | `--keys`, then the title's cache ([Startup and keys](#startup-and-keys)) |
| `dlc` | DLC cutoff: `basegame` or `0`; a build number, meaning the last pack at or before it; or a pack `nameHash` | `--dlc`, then every pack |

A `nameHash` not in `dlclist.xml` is a warning and mounts every pack. The result
is `gameRoot`, `edition`, `dlc` (as given, or `"all"`), `packs` (DLC packs
mounted), `roots` (mount roots) and `warnings`. `edition` is `gta-v-enhanced`
when the root holds `GTA5_Enhanced.exe`, else `gta-v-legacy`; a Legacy mount
skips the Enhanced-only packs, named with `g9ec`, and warns with their count.

```
-> {"id":2,"cmd":"mount","game":"D:\\Games\\...\\Grand Theft Auto V","dlc":"2699"}
<- {"id":2,"ok":true,"gameRoot":"D:\\Games\\...","edition":"gta-v-legacy","dlc":"2699","packs":82,"roots":111,"warnings":["using cached keys (...)"]}
```

### `status`

No parameters. The result is `mounted` plus the fields `mount` returns other
than `warnings`; each is `null` when nothing is mounted.

```
-> {"cmd":"status"}
<- {"ok":true,"mounted":true,"gameRoot":"D:\\Games\\...","edition":"gta-v-legacy","dlc":"2699","packs":82,"roots":111}
```

### `unmount`

No parameters. Drops the mount, the keys and the game root; the result is the
bare envelope.

### `search`

At least one of `pattern`, `ext` and `fileTypes` is required.

| Field | Meaning |
|---|---|
| `pattern` | a case-insensitive substring, or a glob with `*` and `?`, matched against the file's leaf name |
| `fileTypes` | data-file types such as `WEAPONINFO_FILE`, matched against the base game, title update and DLC `content.xml` registries; mounted only |
| `raw` | `true` crawls every `.rpf` on disk, nested ones included, instead of the mounted view; needs a game root (the mounted one, `--game` or `prefs.json`) and no mount |
| `ext` | extensions to keep, an array or a comma-separated string; case and a leading dot are ignored |
| `device` | keep hits whose device prefix (`dlc_mpHeist3CRC` in `dlc_mpHeist3CRC:/...`) contains this, case-insensitive; mounted only |
| `countOnly` | `true` answers without `hits` |
| `limit`, `offset` | a page of the hits |
| `maxHits` | how many hits the sweep holds before it stops: a positive number, default 100,000; `0` is an error and there is no unlimited |

The result is `count` (after `ext` and `device`, before `limit` and `offset`),
`hits`, `truncated`, `maxHits` and `warnings`. A hit has `path`, the logical
path, and `container`, where it resolves: the archive nesting joined with `/`,
after overlay shadowing such as a title update's `dlc_patch`. A `fileTypes` hit
adds `fileType`, `disabled` (the effective state: `false` when any content
change set enables the file) and `pack` (a `nameHash`, or `basegame`).

⚠ `truncated` is the sweep's ceiling, not pagination: a page is a window onto a
complete answer, a truncated sweep an incomplete one. It is decided before `ext`
and `device` filter the hits, so a short filtered list can still be truncated,
and it is `true` whenever the sweep held exactly `maxHits`. A client that
ignores it reports a `count` that is not a total.

```
-> {"id":3,"cmd":"search","fileTypes":["WEAPONINFO_FILE"],"pattern":"navyrevolver"}
<- {"id":3,"ok":true,"count":1,"hits":[{"path":"dlc_mpHeist3CRC:/common/data/ai/weapon_navyrevolver.meta","container":"update/update.rpf/dlc_patch/mpheist3/common/data/ai/weapon_navyrevolver.meta","fileType":"WEAPONINFO_FILE","disabled":false,"pack":"mpHeist3"}],"truncated":false,"maxHits":100000,"warnings":[]}
```

### `packs`

No parameters; needs a mount. The result is `count` and `packs`, in engine
mount order, each with `nameHash`, `deviceName`, `contentType` (`title-update`,
`level`, `compat`, `paid` or `unknown`), `order`, `minorOrder`, `isLevelPack`,
`subPackCount`, `dataFiles` (its registry entry count) and `source`. The
one-shot equivalent is `openrage-cli dlc-list [--dlc <X>] [--json]`, which also
takes what the server does not: `--level <pack>`, which stops at a pack of the
DLC list by the name the list gives it, leaves out the packs listed after it
and refuses a name the list lacks, and `--mode <story|multiplayer>`, which
applies the change sets a pack's setup groups for that mode. It names the packs
left out, the change sets left out and the ones no group states a mode for:
below the table as text, and with `--json` as `leftOut` beside `packs`, and
under `--mode` as `mode` and `changeSets`, each entry a `pack` with its
`leftOut` and `unstated` change sets.

```
-> {"cmd":"packs"}
<- {"ok":true,"count":82,"packs":[{"nameHash":"mpBeach DLC","deviceName":"dlcMPBeach","contentType":"compat","order":1,"minorOrder":0,"isLevelPack":false,"subPackCount":0,"dataFiles":51,"source":"D:\\...\\mpBeach\\dlc.rpf"}]}
```

### `list`

`path` is required; needs a mount. The result is `entries`, the names of the
immediate children of that VFS directory.

```
-> {"cmd":"list","path":"common:/data/ai"}
<- {"ok":true,"entries":["weapons.meta","weaponcomponents.meta"]}
```

### `locate`

`path` is required; needs a mount. The result is `container`, as in a `search`
hit, without reading the file; a path that does not resolve is an error.

```
-> {"cmd":"locate","path":"dlc_mpHeist3CRC:/common/data/ai/weapon_navyrevolver.meta"}
<- {"ok":true,"container":"update/update.rpf/dlc_patch/mpheist3/common/data/ai/weapon_navyrevolver.meta"}
```

### `extract`

`path` is required; needs a mount. `out` writes the file there and answers
`bytes` and `out`; without it, `inline: true` answers `bytes` and `data`, the
file in base64, for a file up to 8 MiB, and a larger one is an error. One of
the two is required. Prefer `out`.

```
-> {"cmd":"extract","path":"dlc_mpHeist3CRC:/common/data/ai/weapon_navyrevolver.meta","out":"C:\\tmp\\nr.meta"}
<- {"ok":true,"bytes":18640,"out":"C:\\tmp\\nr.meta"}
```

### `extract-many`

`paths` (VFS paths) and `outDir` are required; needs a mount; `extract_many` is
an alias. A file lands at `<outDir>/<device>/<relative path>`, the device
without its `CRC` suffix, so aliases share a folder (`commoncrc:` and `common:`
go to `common/`) and each DLC pack keeps its own, as in the engine. The result
is `requested`, `written` (`path`, `out`, `bytes`) and `errors` (`path`,
`error`); a failed file does not stop the batch.

```
-> {"cmd":"extract-many","paths":["commoncrc:/data/ai/weapons.meta"],"outDir":"C:\\tmp\\out"}
<- {"ok":true,"requested":1,"written":[{"path":"commoncrc:/data/ai/weapons.meta","out":"C:\\tmp\\out\\common\\data\\ai\\weapons.meta","bytes":1110684}],"errors":[]}
```

### `shutdown`

No parameters. Answers `{"ok":true}` and the process exits 0.

## Routes that change a pack

Four routes change the clothing content of a project, which is a folder of the caller's own and
never the install. They are named `wardrobe-` and need no mount. Each names the project in `project`
(a `.openrage-proj` path) and, when the project holds more than one clothing content, the one in
`module`. Each takes the right to write the pack first, as a window does, so a pack another OpenRage
holds is refused and not raced, and each is all or nothing: a refusal is the usual
`{"ok":false,"error":"..."}` and changes nothing. A route ends with the pack on disk as it says, so a
client reads `wardrobe.json` to see the result.

| Route | Parameters | Result |
|---|---|---|
| `wardrobe-add` | `from`: files and folders to add, as dropping them on the Wardrobe does (a game file becomes a source at once); or `package`: a garment package folder (`openrage-cli wardrobe package` writes one) | `garments` (ids), `textures`, `cloth`, `unread`, `alreadyHeld`, `leftOut` (`path`, `why`); a package gives `garments` and `notes` |
| `wardrobe-set` | `garments`: ids, and any of `label`, `group`, `ped` (`male` or `female`), `slot`, `position`, `skin` (`universal` or `race`), `heels`, `hairScale`, `hairHeight`, `stickyness`, `audio`, `audio2`, `firstPerson` (each `null` for none), `flags` (the bits as a number), `flagOn` and `flagOff` (the engine's names such as `NOT_IN_CAR`), `propMask`, `renderFlags`, `addTag`, `removeTag`, `remove`: `true` | `garments` (how many), `changed` |
| `wardrobe-fix` | none | `changed`, `sentence`, `report` (`steps`, `left`) |
| `wardrobe-waive` | `code`, `reason`, and `garment` for one garment | `waivers` (how many the pack holds) |

```
-> {"id":4,"cmd":"wardrobe-add","project":"C:\\packs\\demo.openrage-proj","from":["C:\\drop"]}
<- {"id":4,"ok":true,"module":"wardrobe","garments":["a1b2c3d4e5f6"],"textures":1,"cloth":0,"unread":0,"alreadyHeld":[],"leftOut":[]}
-> {"cmd":"wardrobe-waive","project":"C:\\packs\\demo.openrage-proj","code":"lod-missing","reason":"the shop wants it"}
<- {"ok":true,"module":"wardrobe","waivers":1}
```

⚠ A request that changes nothing it could (`wardrobe-set` with no key to set, `wardrobe-add` with a `from` path that is not there or files that hold nothing to add, `wardrobe-waive` with
no reason, a garment the pack does not have) is refused and writes nothing. A build is not a route:
build a project and read it back with `openrage-cli wardrobe build` and `wardrobe verify`.
`crates/tools/or-cli/src/serve.rs` holds a script that adds, sets, fixes and waives over a staged
pack and reads `wardrobe.json` after each request.

## What is not a route

`openrage-cli project` is a set of one-shot commands, not routes: it makes and
changes projects and writes builds, and every route of this protocol reads but the four that change a pack. Its
verbs are `create`, `adopt`, `import`, `add-content`, `add-build`, `set-build`,
`set-file-type`, `remove-build`, `link`, `unlink`, `links`, `template`,
`apply-template`, `check-sharing`, `check-original`, `plan`, `build`, `verify`,
`clean`, `redirects`, `history`, `rollback`, `remove` and `recook`; each
exits 0, 1 for a refusal or a check that failed and 2 for a usage error.

A command that works on one build of a project names it with `--build <id>`.
`--output <id>` is the older spelling of the same flag: it is still read, by
`project`, `wardrobe` and `addonped` alike, and the usage does not show it, so a
script written before the rename runs as it did.

## Scope

One client, the process that spawned the server, served in order, with no
authentication: the server trusts whoever holds its stdio. A server that dies,
from an abort or running out of memory, is a child process to respawn and
`mount` again. [openrage_client.go](openrage_client.go) is a
minimal Go client that pings, mounts, searches and shuts down. An AI agent
reaches the same routes as tools through `openrage-cli mcp`
([MCP_Server.md](MCP_Server.md)).
