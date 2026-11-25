# GTA VI script decompiler

OpenRage reads a compiled GTA VI script and writes it as C#-like source: classes, methods,
`await`, `foreach` and structured statements where the code proves them, and the lower form
where it does not. The command line, the MCP server and the app all read through
`crates/io/or-io/src/gta6/script/`, and nothing of a program is ever run.

## What is read

A GTA VI script is a program for a managed virtual machine: 64-bit cells, objects with a type at
their head, arrays, boxing, virtual calls by slot, delegates and compiler-made state machines.
Two front ends give the passes the same form.

| Front end | Input | Code |
|---|---|---|
| Compiled program | a deciphered program, as a file or as an entry of an RPF9 archive | `program.rs` (the container), `opcode.rs` and `decode.rs` (the instruction set), `lift.rs` (stack code to expressions and blocks) |
| Lifted listing | the text another decompiler writes from the bytecode, one statement an instruction | `listing.rs` |

Of a compiled program the reader takes the container (functions, types with their fields and
method records, imports, the functions the engine calls by name) and every instruction of its
code. A string constant prints in the code that uses it; `--strings` lists none yet. A file that
is not a program and one whose code does not decode are refused with an error, and a function the
lifter cannot settle is written as a comment that says why, with the rest of the program around
it.

A second, older reader of the same bytes prints a plain instruction listing
(`openrage-cli script compiled`, `compiled.rs` with its own `disasm.rs`). It knows the
interpreter's two whitening rules and names a native by the address table the user's listings
gave; it lifts nothing.

⚠ A compiled program stores **no identifier as text**, only hashes, and calls a native by the
address of its handler. Every name in the output therefore comes from a list the user supplies,
and a hash no list names stays a hash.

## Levels

`--level` says how far the output is taken from the bytecode.

| Level | What is written |
|---|---|
| `lifted` | the front end's own form in a listing's syntax; no pass runs |
| `structured` | functions of `if`, loops and `switch`, with a labelled `goto` only where none fits; no source construct is rebuilt |
| `source` | classes with their members, and every source construct a pass proves |

`script decompile` and `script export` default to `source`. `script lifted` defaults to the four
first passes (`values`, `types`, `structure`, `names`) at `structured`, which is what it printed
before the others existed.

## Passes

`openrage-cli script passes` prints every pass with the level it belongs to, and `--no <pass>`
leaves one off, so the code it would have rebuilt stays in the lower form. The rule every
reconstruction follows: it fires only when its whole pattern matches, it keeps a way back to the
form it replaced, and it can be switched off by name. A pass that is unsure leaves the code as it
was.

- **Values and flow**: `values`, `escape`, `forward`, `dead-loads`, `constants`, `coalesce`,
  `interproc`, `aggregates` fold temporaries into their uses, forward stores to loads, merge
  copies and carry signatures across calls.
- **Kinds**: `types`, `kinds`, `signs` give a variable a declared type where every read and write
  agrees; a variable of no proved kind is declared `Cell`, and a 32-bit reading of it is written
  as a cast.
- **Statements**: `structure`, `gotos`, `switch-on`.
- **Names**: `names`, `hash-text`, `native-params`, `role-names`.
- **Classes**: `fields`, `classes`, `receivers`, `constructors`, `operators`.
- **Compiler expansions**: `await`, `lambdas`, `delegates`, `foreach`, `compound`,
  `initialisers`. A lambda is found and still written by its function's name, never as `=>`.

These names are accepted and rebuild nothing yet: `properties`, `iterators`, `wait`,
`local-functions`, `events`, `using`, `lock`, `try`, `for`, `concat`, `ref-out`, `nullable`,
`tuples`. Their constructs print in the lower form, and so does what `null-ops` looks for: the
shape it reads as `??` does not reach it.

## The project

A type, a field or a signature is often stated by another program than the one that uses it.
`script export` reads every program of a build folder, an archive or a folder of deciphered
programs, and with `-o <folder>` writes:

- `project.json`: what the programs state of each other (types, layouts, signatures, calls),
  never a name;
- one `.cs` per program, decompiled with that project;
- `scripts.json`: what was read, where each program is, and with which lists, level and passes.

`script decompile <src> --project <file>` decompiles one program with a project an export wrote.
Without one a program is decompiled alone, and a function that needs what another program states
is written as a comment saying so. Output is the same bytes from run to run.

## Names

OpenRage ships no natives list and no names. The first lines of a decompiled source say which
lists named things; `disasm` and the facts of a program consult no list.

| List | Form | Names |
|---|---|---|
| Natives list (`--natives-db`, else `OPENRAGE_NATIVES`, else what `script download-natives` fetched) | JSON, or `0xHASH NAME` text | a native by its 64-bit hash, with its parameters; for a compiled program only together with a handler table, and silently not otherwise |
| Names list (`--names`) | one `0xHASH name` a line, or a tab-separated table under a row of column names | types, programs, modules, methods, fields, and the text of hash constants |
| Handler table (`--names`) | a table whose first columns are `hash` and `rva` | the hash of the native at each handler address, which is how a natives list reaches a compiled program |

A row whose hash is the hash of its name proves itself and names whatever that hash function
names; any other row is the user's own label. Without `--names`, the command line reads every
file of the `gta6_script_names` folder of the configuration directory that reads as a names list.
The app reads the same folder by another rule: a file whose name starts with `natives` is a
natives list there, and every other file a names list.

## What cannot be recovered

- The names of locals, parameters, and of every type, member and native no list holds. They print
  by offset, slot or hash: `local_3`, `p0->f_10`, `Method_4`, `func_000100`.
- Comments, formatting, and the choice between source forms that compile to the same code.
- The initial values of globals and statics: the container holds none.
- A construct whose expansion a pass does not prove. A state machine whose entry does more than
  load its fields, or whose frame variable is written on the way to an `await` and read after it,
  stays a state machine.
- Exception regions have no tree form: a function that holds one is written flat.

## How it is checked

- `exec_tests.rs` runs a function's blocks on made-up inputs before the passes and after each of
  them and compares every call, every write through a pointer and what is returned, over
  hand-written functions. ⚠ It does not check the printer, and a statement in a block no input
  reaches is compared by nothing.
- `exec_corpus_tests.rs` (slow tier) runs that check over every function of the folder of
  listings `OPENRAGE_LIFTED_LISTINGS` names, and lifts every compiled program of the folder
  `OPENRAGE_GTA6_SCRIPTS` names and holds each function to what its listing does.
- `reparse.rs` reads printed source back to the tree it was printed from, over hand-written
  functions; its run over a folder of listings does not yet finish in bounded memory.
- `crates/io/or-io/tests/gta6_script_refuses_hostile_input.rs` holds the readers to an error,
  in bounded time and memory, on truncated and corrupted programs.

## Command line

`openrage-cli script --help` has the full usage. A source is a deciphered program file, told by
its first bytes, or an archive and one entry of it (`<archive.rpf> [inner/path] --entry <e>`,
with the key snapshot `rpf9` takes).

| Command | Does |
|---|---|
| `script <src> [--functions\|--natives\|--strings] [--json]` | the facts of one program |
| `script disasm <src> [--function <f>]` | an instruction listing with targets resolved |
| `script decompile <src> [--function <f>] [--level <l>] [--no <pass>] [--project <file>]` | source |
| `script check <file-or-folder>` | decodes each program and fails on one that does not |
| `script list <archive.rpf>` | the programs an archive holds |
| `script export <build\|archive\|folder> [-o <folder>]` | every program, with its project |
| `script passes` | the passes and their levels |
| `script lifted <listing.ysc.cs>` | source from a lifted listing |
| `script compiled <file>` | the older instruction listing |

[MCP_Server.md](../reference/MCP_Server.md) says how an agent reads a script a function at a
time through the `cli` tool.

## In the app

A compiled script opens read-only as a **GTA VI Script** document, from an archive entry of the
Files tree, decompiled on the load worker, or from a deciphered file. It is decompiled alone,
with no project. The text editor shows the
source with highlighting, **Go to definition** and **Find references** follow the name under the
caret within the script, the outline lists its classes and functions, and the inspector states
the program's facts, the lists that named things, the level, and the instructions of the function
picked. **Open names folder** opens the folder the lists are read from. A body the
container reader refuses opens as the older instruction listing where that reader takes it.
