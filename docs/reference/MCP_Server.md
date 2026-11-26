# MCP server

How an AI agent drives OpenRage with no window: `openrage-cli mcp`, a Model
Context Protocol server over stdio, how a client is given it, and the tools it
offers.

It is the MCP adapter of [R43](../direction/Roadmap.md#r43-the-external-surface-and-open-with),
written in `crates/tools/or-cli/src/mcp.rs` over the routes of
[the `serve` protocol](CLI_Server_Protocol.md) and the commands of
`openrage-cli`.

## Install

```
openrage-cli mcp install
```

It registers the binary it is run from with each client it finds, and prints
the entry any other client takes. Restart the client to see the tools.

| Client | How it is registered |
|---|---|
| Claude Code | through its own `claude mcp add`, at user scope, replacing an earlier `openrage` |
| Claude Desktop | `claude_desktop_config.json` gains `openrage` under `mcpServers` and keeps the rest |
| Cursor | `mcp.json` in its `.cursor` folder, the same way |
| any other | start `openrage-cli mcp` and speak MCP over its stdio |

```json
{ "mcpServers": { "openrage": { "command": "C:\\OpenRage\\openrage-cli.exe", "args": ["mcp"] } } }
```

- `openrage-cli mcp install cursor` registers with the clients it names only:
  `claude-code`, `claude-desktop`, `cursor`.
- A client that is not on the machine is reported as not found. A
  configuration file that is not a JSON object is reported and left as it is.
- Nothing else is needed: `mount` with no `game` takes the install the app has
  selected, and keys are resolved as
  [`serve` resolves them](CLI_Server_Protocol.md#startup-and-keys).

⚠ The entry holds the binary's full path, so a client keeps starting the old
one after OpenRage moves or a second copy is installed. Run `mcp install`
again from the copy to use.

## Transport

- Newline-delimited JSON-RPC 2.0: requests on stdin, answers on stdout, one
  object per line, logs on stderr. A notification is not answered.
- It answers `initialize`, `ping`, "tools/list" and "tools/call", and offers
  tools only: no resources and no prompts. Any other method is error -32601,
  and a tool it does not have is error -32602.
- `initialize` answers the protocol revision the client asked for when it is
  one of `PROTOCOLS` in `crates/tools/or-cli/src/mcp.rs`, and the newest of
  them otherwise.
- One request at a time, in the order received. The mount lives as long as the
  process, which exits at stdin EOF.

## Tools

A tool that fails answers `isError` true with the reason as its text.

| Tool | Does |
|---|---|
| `mount`, `status`, `unmount` | mount an install once for the tools below, say which is mounted, drop it |
| `search`, `packs`, `list`, `locate` | find files by name, extension or data-file type; list the DLC packs; list a folder; say which archive a file resolves to |
| `extract`, `extract-many` | copy files out of the mounted install to paths the caller names |
| `wardrobe-add`, `wardrobe-set`, `wardrobe-fix`, `wardrobe-waive` | change the clothing pack of a project |
| `rpf9-list`, `rpf9-find`, `rpf9-read`, `rpf9-census` | list a GTA VI archive, find entries across an install, read one entry deciphered, count an install |
| `rpf9-patch`, `rpf9-fits` | write a patched copy of a GTA VI archive, its table byte-identical; say whether a replacement fits, writing nothing |
| `rpf9-copy`, `rpf9-replace`, `rpf9-remove`, `rpf9-rename`, `rpf9-defrag` | write a new GTA VI archive from one: a copy, or a rebuilt archive, which the game refuses |
| `cli` | run any `openrage-cli` command |
| `shots` | draw the editor from a script and answer the pictures |

The first four rows are the routes of `serve` under their own names, with the
parameters and results [CLI_Server_Protocol.md](CLI_Server_Protocol.md#routes)
gives; the result is the tool's text, as JSON without the `ok` field. `ping`
and `shutdown` are the transport's and are not tools.
`every_route_but_the_transports_own_is_a_tool_of_the_mcp_server` in
`crates/tools/or-cli/tests/the_external_surface_is_what_it_documents.rs` fails
when a route has no tool.

### GTA VI archives

Each `rpf9-` tool runs the `openrage-cli rpf9` verb of the same name in a
process of its own and answers as `cli` does;
`rpf9-read` is `rpf9 extract --decrypt`, and `rpf9-list` answers JSON. A parameter is the verb's option of
the same name, and `archive`, `folder`, `outDir` and `inner` are its
positional words. They are declared in `crates/tools/or-cli/src/mcp_gta6.rs`.

- `keys` names a key snapshot; without it the verb finds one as
  `openrage-cli rpf9` does, and `exe` opens a masked one.
- `entry` is an entry's recovered name, its hash or its decimal index, as
  `rpf9-list` gives them; `inner` walks nested archives.
- `rpf9-census` writes only counts, into the `out` folder it is given.
- `rpf9-replace` with `resource` true replaces an entry with a 20-byte row,
  a paged resource, by the loose form `with` names.
- `rpf9-patch` puts the body `with` names in place of `entry`, chunk by
  chunk, and `rpf9-fits` says, for each chunk, whether it fits and by how many
  bytes one does not; a body of another length than the row's is refused.

A write tool writes one of two kinds of archive, and its answer says which.
A patched copy keeps every byte of the table, so the table's signature still
matches and no check read in the game's code rejects it; the game has not
been run with one. A rebuilt archive, one whose table `rpf9-replace`,
`rpf9-remove`, `rpf9-rename` or `rpf9-defrag` laid out again, is refused by
the game, which checks a signature over the table when it mounts an archive,
and only the game's maker can sign one.

⚠ A rebuilt archive is a structural result and never a mod the game loads:
a change that fits its entry's chunks goes through `rpf9-patch` instead. Each
write tool writes a new file at `out`, and the source is never changed.

### `cli`

`args` is the command line after `openrage-cli`, one word per item; `cwd` is
the folder to run in; `timeoutSeconds` stops the command, 600 when omitted.
The answer is the exit code, then what the command wrote to stdout and to
stderr, each cut to its last 64 KiB with the count of bytes left out. An exit code other
than 0 is the tool's error.

Each call is a process of its own, so nothing is mounted between calls and a
file is named by its absolute path. The tool's description lists the commands,
from `COMMANDS` in `crates/tools/or-cli/src/lib.rs`, so a new command is
offered without a change to the server; `["help"]` says what each does and
`[COMMAND, "--help"]` gives its usage.

A compiled GTA VI script decompiles to more than an answer keeps, so it is read
a part at a time. No tool of its own is offered for a script.

| `args` | What it answers |
|---|---|
| `["script", "list", ARCHIVE]` | the programs of an archive |
| `["script", FILE, "--functions", "--json", "--offset", N, "--limit", N]` | one page of a program's functions as a JSON array, one function a line; stderr says which functions the page holds and the `--offset` that prints the next |
| `["script", "decompile", FILE, "--function", NAME]` | one function: its source and the type that holds it, without the program's other types; `--with-classes` writes those too |
| `["script", "decompile", FILE, "-o", OUT]` | the whole program, written to a file |

Choose `--limit` so that a page fits an answer. A `project.json` beside FILE
is read without being named, which takes seconds when it is the project of a
whole build; `--alone` decompiles without it, and `--project` names another.

⚠ An answer keeps the last 64 KiB of each stream. A list or a source asked for
whole comes back without its first part, its header among it, and a list cut so
is no longer JSON: ask for a page, or write to a file with `-o` and read that.

### `shots`

`steps` is a script of
[the steps `just shots` reads](../guides/Development_Workflow.md#screenshots),
one per line; `outDir` is where the PNGs go, a new folder under the temp folder
when omitted; `timeoutSeconds` is as for `cli`. It runs the `openrage` editor
beside the binary with no window, on a scratch configuration, and answers what
the run printed and each `shot` as an image, up to 8 MiB of them in one answer;
a picture past that is answered by its path alone. It needs a GPU adapter.

⚠ Each call is a new editor, so a script states its whole flow: a second call
does not continue the first.

## What a tool can change

- The mounted install is only read.
- `extract` and `extract-many` write the paths the caller names, and the
  `wardrobe-` tools change the pack of the project the caller names
  ([routes that change a pack](CLI_Server_Protocol.md#routes-that-change-a-pack)).
- `rpf9-read` and `rpf9-census` write into the folder the caller names,
  `rpf9-fits` writes nothing, and each other GTA VI write tool writes a new
  archive at `out`, never its source.
- `cli` writes what its command writes: an `rpf`, `project`, `texture` or
  `navmesh` command given a path writes there, an install's file included.
- A click in a `shots` script reaches every command of the editor.

There is no authentication: the server trusts whoever holds its stdio, as
`serve` does, so the client's own permission prompt is the only one.
