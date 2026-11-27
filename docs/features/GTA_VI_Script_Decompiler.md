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
code. A string constant prints in the code that uses it, and `--strings` lists the strings the
code pushes. A file that
is not a program and one whose code does not decode are refused with an error, and a function the
lifter cannot settle is written as a comment that says why, with the rest of the program around
it.

`openrage-cli script compiled` prints a plain instruction listing of the same bytes with none of
the program's tables (`compiled.rs` and `disasm.rs`, which read the one instruction table of
`opcode.rs`), so it lists a body the container reader refuses. It names a native by the address
table the user's listings gave; it lifts nothing.

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
before the others existed; `--no <pass>` alone leaves one of those four off, and only `--level`
moves it to another level. Its first lines say which of the two it ran.

## Passes

`openrage-cli script passes` prints every pass with the level it belongs to and marks the names
that rebuild nothing yet, and `--no <pass>`
leaves one off, so the code it would have rebuilt stays in the lower form. The rule every
reconstruction follows: it fires only when its whole pattern matches, it keeps a way back to the
form it replaced, and it can be switched off by name. A pass that is unsure leaves the code as it
was.

- **Values and flow**: `values`, `escape`, `forward`, `dead-loads`, `constants`, `coalesce`,
  `interproc`, `aggregates` fold temporaries into their uses, forward stores to loads, merge
  copies and carry signatures across calls. A call through a virtual slot yields the kind every
  function the slot holds down its line of types proves, and none where one of them proves
  nothing.
- **Kinds**: `types`, `kinds`, `signs` give a variable a declared type where every read and write
  agrees; a variable of no proved kind is declared `Cell`, and a 32-bit reading of it is written
  as a cast. A field of one byte that every program only ever stores a truth to and only tests
  for zero is a `bool`, and so is the result of a native that every program only tests, or only
  hands back from a function every call of which only tests it. A pointer, like a reference, is
  compared with `null`. A variable every
  assignment of which makes an array is an array of that element type. A value of two cells that
  the code only takes from natives, compares and hands back to them is declared `Handle`, a word
  the header explains. A value of several cells that a `box<T>` of it, or a method of the value
  type `T` called on its address, names is of type `T`, and bytes taken out of a whole variable
  are its field, read as the unsigned number they make up. A parameter every call of which, from
  functions that every call names, passes an `int` widened by its sign is read as an `int`. A
  constant of no proved kind whose bits are those of a round float keeps its bits and has the
  float written beside it as a comment, `0x3F800000 /* 1.0f */`: the bits alone prove no kind.
- **Statements**: `structure`, `gotos`, `switch-on`, `for`.
- **Names**: `names`, `hash-text`, `native-params`, `role-names`. `hash-text` writes a constant
  as the text of its hash only in a cell a user's list says holds hashes; a variable handed to
  such a cell holds hashes, so the labels of a switch on it are written as texts too.
- **Classes**: `fields`, `classes`, `receivers`, `constructors`, `operators`. An access through a
  variable the kinds pass proves holds an object of a script type is a field of that type, where
  the bytes lie in the type and no field the layouts state overlaps them; such a field is
  declared in its class with the width every access of it takes and no proved kind. A
  reference field is of the script type of the objects stored in it, where they all lie on one
  line of types. `script
  export` reads every program's code a second time once the project is settled, so that the
  accesses through such variables and through a member's receiver add to the layouts too. Two
  whole variables compared byte by byte are compared with `==`.
- **Compiler expansions**: `await` reads a kick-off and the step function it calls as one
  method, `async Task<T>` where its state machine gives the task a value and `async void`
  otherwise; `lambdas`, `delegates`, `events`, `foreach`, `using`, `lock`,
  `compound`, `initialisers`, `null-ops`, `concat`, `ref-out`. A lambda whose body is one value or
  a few statements, bound once and outside every loop, is written as `=>` where it is bound; any
  other is written by its function's name. `events`, `concat`, `using` and `lock` read a call
  only by the name a user's list gives it, so without such a list they rebuild nothing. An
  address handed to a call is written `ref x` only where it is proved to reach `x` alone, and
  `&x` otherwise. `boxing` writes `object.Equals(box<T>(a), box<T>(b))` as `a == b` for a type
  whose values are equal exactly where their bits are (never a float), and
  `Convert.ToInt32(box<T>(x))` as `(int)x` where no value of `T` lies past an `int`'s range. A
  clear of every byte of a variable is written `x = default;`.

These names are accepted and rebuild nothing yet: `properties`, `iterators`, `wait`,
`local-functions`, `nullable`, `tuples`, `try`. Their constructs print in the lower form.

## The project

A type, a field or a signature is often stated by another program than the one that uses it.
`script export` reads every program of a build folder, an archive or a folder of deciphered
programs, and with `-o <folder>` writes:

- `project.json`: what the programs state of each other (types, layouts, signatures, calls),
  never a name;
- one `.cs` per program, decompiled with that project;
- `scripts.json`: what was read, where each program is, and with which lists, level and passes.

`script decompile <src> --project <file>` decompiles one program with a project an export wrote,
and a `project.json` beside a program file is read without being named unless `--alone` is given.
Without one a program is decompiled alone, and a function that needs what another program states
is written as a comment saying so; the first lines of the source say which it was. Output is the
same bytes from run to run.

## Names

OpenRage ships no natives list and no names. The first lines of a decompiled source say which
lists named things; `disasm` and the facts of a program consult no list.

| List | Form | Names |
|---|---|---|
| Natives list (`--natives-db`, else `OPENRAGE_NATIVES`, else what `script download-natives` fetched) | JSON, or `0xHASH NAME` text | a native by its 64-bit hash, with its parameters; for a compiled program only together with a handler table, and silently not otherwise |
| Names list (`--names`) | one `0xHASH name` a line, or a tab-separated table under a row of column names | types, programs, modules, methods, fields, and the text of hash constants |
| Word list (`--names`) | one word a line with no hash | candidates: a word names a type, a field or a module only where its own hash is the one the program holds, and a method only while the list is short enough that a chance match of a 32-bit hash is unlikely |
| Handler table (`--names`) | a table whose first columns are `hash` and `rva` | the hash of the native at each handler address, which is how a natives list reaches a compiled program |

A row whose hash is the hash of its name proves itself and names whatever that hash function
names; any other row is the user's own label. Without `--names`, the command line reads the
`gta6_script_names` folder of the configuration directory, its files in the order of their names:
a file whose name starts with `natives` is a natives list where it reads as one and a names list
otherwise, any other file a names list where it reads as one and a natives list otherwise, and a
file that reads as neither is said on stderr and passed over. A file whose name starts with a dot
and `project.json` are no lists. The app reads the same folder by the same rule, except that it
also tries a file that holds JSON as a natives list first, and it takes a `project.json` from that
folder where none lies beside the script.

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
  reaches is compared by nothing. ⚠ With `escape` on, the passes and the check both take an
  address handed to a call to reach the variable it is the address of and no local the code
  proves to be another variable: both runs are made on that premise, so the check does not test
  it. A temporary handed out is known in the events by the order it was first handed out, as an
  object is, so two forms that name their temporaries apart compare alike. A parameter the
  project proves an `int` is made up as one, and a call that hands such a parameter another cell
  fails the check.
- `exec_corpus_tests.rs` (slow tier) runs that check over every function of the folder of
  listings `OPENRAGE_LIFTED_LISTINGS` names, and lifts every compiled program of the folder
  `OPENRAGE_GTA6_SCRIPTS` names and holds each function to what its listing does; it also counts
  the sites of three opcodes that only this suite checks by what holding their function came to. Over the listings and the project
  `OPENRAGE_LIFTED_PROJECT` names it holds the premise above to an independent measure: the
  project states, for each parameter cell of each function, how many bytes from an address handed
  in that cell the function and the functions it hands it on to may touch, and every address of a
  local handed to a function is held to the width the passes take it to reach. An `async`
  method made of a kick-off and its step function is read back into the two, and each is held to
  the function it was made of.
- `reparse.rs` reads printed source back to the tree it was printed from, over hand-written
  functions, with a bound on the statements and operands of one function; ⚠ its run over a folder
  of listings (slow tier) has not been run to its end.
- `crates/io/or-io/tests/gta6_script_refuses_hostile_input.rs` holds the readers to an error,
  in bounded time and memory, on truncated and corrupted programs and on valid containers whose
  code is hostile.

## Command line

`openrage-cli script --help` has the full usage. A source is a deciphered program file, told by
its first bytes, or an archive and one entry of it (`<archive.rpf> [inner/path] --entry <e>`,
with the key snapshot `rpf9` takes).

| Command | Does |
|---|---|
| `script <src> [--functions\|--natives\|--strings] [--json] [--offset <n> --limit <n>]` | the facts of one program; `--functions` by pages |
| `script disasm <src> [--function <f>]` | an instruction listing with targets resolved |
| `script decompile <src> [--function <f>] [--level <l>] [--no <pass>] [--project <file>\|--alone] [--with-classes]` | source; with `--function`, the types that hold no code of it are left out unless `--with-classes` |
| `script check <file-or-folder>` | decodes each program and fails on one that does not |
| `script list <archive.rpf>` | the programs an archive holds |
| `script export <build\|archive\|folder> [-o <folder>]` | every program, with its project |
| `script passes` | the passes and their levels |
| `script lifted <listing.ysc.cs>` | source from a lifted listing |
| `script compiled <file>` | the instruction listing read with none of the program's tables |

[MCP_Server.md](../reference/MCP_Server.md) says how an agent reads a script a function at a
time through the `cli` tool.

## In the app

A compiled script opens read-only as a **GTA VI Script** document, from an archive entry of the
Files tree, decompiled on the load worker, or from a deciphered file. It is decompiled with the
`project.json` beside the file, else the one in the names folder, else alone, and the inspector
says which. A program with more than a mebibyte of code opens with its `main` alone decompiled:
picking another function in the outline decompiles it, and the inspector offers every function.
The text editor shows the
source with highlighting, **Go to definition** and **Find references** follow the name under the
caret within the script, the outline lists its classes and functions, and the inspector states
the program's facts, the lists that named things, the level, and the instructions of the function
picked. **Open names folder** opens the folder the lists are read from. A body the
container reader refuses opens as the plain instruction listing where that reader takes it.
