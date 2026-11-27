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
  nothing. `wide-joins` makes a value of several cells carried to a join in a row of one-cell
  temporaries one variable, and `repeats` sends a test that repeats the one its block was
  entered by the way that one went.
- **Kinds**: `types`, `kinds`, `signs` give a variable a declared type where every read and write
  agrees; a variable of no proved kind is declared `Cell`, and a 32-bit reading of it is written
  as a cast. A field of one byte that every program only ever stores a truth to and only tests
  for zero is a `bool`, and so is the result of a native that every program only tests, or only
  hands back from a function every call of which only tests it. A pointer, like a reference, is
  compared with `null`. A variable every
  assignment of which makes an array is an array of that element type. A value of two cells that
  the code only takes from natives, compares and hands back to them is declared `Handle`, a word
  the header explains, and so is one copied whole from or into a `Handle`, or compared with one, or
  compared as the runtime compares a `System.Guid`. A value of several cells that an instruction storing, making or unboxing
  the whole value names, a `box<T>` of it, or a method of the value
  type `T` called on its address, or a builtin's constructor handed its address, names is of type
  `T`, and bytes taken out of a whole variable
  are its field, read as the unsigned number they make up. A parameter every call of which, from
  functions that every call names, passes an `int` widened by its sign is read as an `int`. A
  constant of no proved kind whose bits are those of a round float keeps its bits and has the
  float written beside it as a comment, `0x3F800000 /* 1.0f */`: the bits alone prove no kind.
- **Statements**: `structure`, `gotos`, `switch-on`, `for`, `conditions`. `conditions` writes a
  truth chosen by an `if` as one condition, `x = a && b`, and an `if` alone inside another as
  `if (a && b)`.
- **Names**: `names`, `hash-text`, `native-params`, `role-names`. `hash-text` writes a constant
  as the text of its hash only in a cell a user's list says holds hashes; a variable handed to
  such a cell holds hashes, so the labels of a switch on it are written as texts too.
  `role-names` names a variable by what a list names: the native or method whose result it
  holds, the native parameter it is passed as, the field it is read from or the one field it is
  only ever stored to, the type of the object it holds.
- **Classes**: `fields`, `classes`, `receivers`, `constructors`, `operators`. An access through a
  variable the kinds pass proves holds an object of a script type is a field of that type, where
  the bytes lie in the type and no field the layouts state overlaps them; such a field is
  declared in its class with the width every access of it takes and no proved kind. A
  reference field is of the script type of the objects stored in it, where they all lie on one
  line of types. `script
  export` reads every program's code a second time once the project is settled, so that the
  accesses through such variables and through a member's receiver add to the layouts too. Two
  whole variables compared byte by byte are compared with `==`. `properties` writes a call of a
  runtime type's accessor `get_X` or `set_X` on its receiver as `x.X` and `x.X = v`; a script's
  own function of such a name stays the method it is, declared and called as one. With
  `fields` on, a field is written `p.f_10` after a variable of one cell, through what it points
  at, and after a variable of several cells, of its own bytes; the bytes of a variable of one
  cell are `(&p)->f_10`, or the variable shifted down, `(byte)(p >> 32)`, and `->` stays after a
  global and after an expression. `runtime` writes a call of the runtime's own library that a
  user's list names, and that a table of the library's public members holds, as C# writes it: a
  method on its receiver (`d.Clear()`), a static property (`Task.CompletedTask`), a construction
  as `new T(a)`; any other stays a call. Before its first function a source declares each global
  and static its code names, of the kind the program or the project states of it, else `Cell`.
  An enum declares a member for each value of it a list names, one value a name, and a value of
  it stored to a field or switched on is written `E.Member` only where that member is declared.
- **Compiler expansions**: `await` reads a kick-off and the step function it calls as one
  method, `async Task<T>` where its state machine gives the task a value, `async Task` where the
  kick-off hands back its builder's task and `async void` otherwise, and writes what is awaited in place, `await f(a)`, where the variable it was kept in
  is used nowhere else; a state machine whose step function is read into its kick-off is left
  out where the project holds the program, no program of it imports this one and nothing else
  written names the machine; `lambdas`, `delegates`, `events`, `foreach`, `using`, `lock`,
  `compound`, `initialisers`, `null-ops`, `concat`, `ref-out`. A lambda bound once and outside
  every loop, whose body jumps nowhere back, is written as `=>` where it is bound, whatever its
  length; any other is written by its function's name. A closure class every lambda of which is
  written in place, and whose captured fields are each given a variable that keeps its value, is
  left out, its fields read as those variables; `--no lambdas` writes it. `events`, `concat`, `using` and `lock`
  read a call only by the name a user's list gives it, so without such a list they rebuild
  nothing. An
  address handed to a call is written `ref x` where the callee is proved to reach `x` alone and
  keep no address, `out x` where it is also proved to write every byte of `x` before it reads any,
  on every way through (a variable made zero just before such a call as `F(out var x)`), and `&x`
  otherwise. A head keeps such a parameter a pointer: no caller is seen where a head is written.
  `boxing` writes `object.Equals(box<T>(a), box<T>(b))` as `a == b` for a type whose values are
  equal exactly where their bits are (never a float), and
  `Convert.ToInt32(box<T>(x))` as `(int)x` where no value of `T` lies past an `int`'s range. A
  clear of every byte of a variable is written `x = default;`.

These names are accepted and rebuild nothing yet: `iterators`, `wait`,
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
`--project auto` builds the project in memory first, as an export builds it, of every program of
the folder the program file lies in, or of the archive an `--entry` is read from and the archives
nested in it; stderr says how far each of its two passes is, it reads at most 4 GiB, and it writes
no file. Without a project a program is decompiled alone, and a function that needs what another
program states is written as a comment saying so; the first lines of the source say which it was.
Output is the same bytes from run to run.

An export works on `--threads` threads (8 by default), a program at a time on each, and starts the
largest programs first in each pass, so that none is left to run alone once the rest are done;
the functions of one program are written by one thread.

## Names

OpenRage ships no natives list and no names. The first lines of a decompiled source say which
lists named things; `disasm` and the facts of a program consult no list.

| List | Form | Names |
|---|---|---|
| Natives list (`--natives-db`, else `OPENRAGE_NATIVES`, else what `script download-natives` fetched) | JSON, or `0xHASH NAME` text | a native by its 64-bit hash, with its parameters; for a compiled program only together with a handler table, and silently not otherwise |
| Names list (`--names`) | one `0xHASH name` a line, or a tab-separated table under a row of column names | types, programs, modules, methods, fields, and the text of hash constants |
| Word list (`--names`) | one word a line with no hash | candidates: a word names a type, a field or a module only where its own hash is the one the program holds, and a method only while the list is short enough that a chance match of a 32-bit hash is unlikely |
| Handler table (`--names`) | a table whose first columns are `hash` and `rva` | the hash of the native at each handler address, which is how a natives list reaches a compiled program |
| Function rows (`--names`) | one `program:offset Name` a line; the program as a hash of up to eight digits, as `program_` and its eight digits, or by its name; the offset in hex after `0x` or `func_` | a function, on trust |
| Enum members (`--names`) | a table under the columns `enum_type_hash64`, `member` and `value` | a value of an enum, under either reading of its sign |
| Enum texts (`--names`) | a table of texts with an `enum_type_hash64` column | a value of an enum by the text its hash is of, where the text is a name and no member row names the value |
| Labels (`--labels`) | one `0xHASH _LABEL` a line (`NS::_LABEL` allowed), or a table whose columns name `hash` and `label` | guesses: a native no natives list names, written by its label, which opens with an underscore, with no parameter or result |

A row whose hash is the hash of its name proves itself and names whatever that hash function
names; any other row is the user's own label. A type's record hashes its full runtime name, so a
proved type name is written in C#'s spelling: a nested type after a `.` in place of `+`, and an
instance of a generic type as that type and its arguments, the arity mark dropped and the
arguments of a nested type given to the generic type it is in (`Map<K, V>.Keys`); a name of any
other mark is counted and not written. Without `--names`, the command line reads the
`gta6_script_names` folder of the configuration directory, its files in the order of their names:
a file whose name starts with `natives` is a natives list where it reads as one and a names list
otherwise, any other file a names list where it reads as one and a natives list otherwise, and a
file that reads as neither is said on stderr and passed over. A file whose name starts with a dot
and `project.json` are no lists. A file whose name holds `candidate` holds guesses, whose rows a
loader cannot tell from proved ones, so the folder passes it over and says so; `--names` reads it.
A file whose name holds `labels` holds native labels, which are guesses too: the folder passes it
over and says so, and only `--labels` reads one. A label must open with `_` and be one name, or
the file is refused, and of several labels files a later one's label stands in place of an
earlier one's. A source's first lines say how many natives each labels file labelled, and its
natives line counts the calls written by a label apart from those a list names. A label names no
variable, and a native written by one never counts as named.
A list every row of which proves itself by its hash is read before every other list of the
folder, so that where a label given on trust and a proof name one hash differently, the label's
list is the one refused. The first lines of a source name each list read with how many hashes,
cells, indices, functions or words it named that no list before it did. A builtin's name a later
list gives stands in place of the one an earlier list gave, and the source's names line says how
many each list replaced; one list that names a builtin twice is refused. The app reads the same folder by the same rules,
except that it also tries a file that holds JSON as a natives list first, it takes a
`project.json` from that folder where none lies beside the script, and it reads the folder's
labels files only while **Label natives from the labels files (guesses)** is on in the inspector,
which is off at every start.

## What cannot be recovered

- The names of locals, parameters, and of every type, member and native no list holds. They print
  by offset, slot or hash: `local_3`, `p0.f_10`, `Method_4`, `func_000100`.
  A label `--labels` gives a native is a guess, not a recovered name, and is written as one, opening
  with an underscore.
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
| `script decompile <src> [--function <f>] [--level <l>] [--no <pass>] [--project <file>\|auto\|--alone] [--labels <file>] [--with-classes]` | source; with `--function`, the types that hold no code of it are left out unless `--with-classes` |
| `script check <file-or-folder>` | decodes each program and fails on one that does not |
| `script list <archive.rpf>` | the programs an archive holds |
| `script export <build\|archive\|folder> [-o <folder>] [--threads <n>] [--labels <file>]` | every program, with its project |
| `script passes` | the passes and their levels |
| `script names-search <program\|folder>... [--words <file>]... [-o <file>]` | a names list of the type, field and program names a word search proves, each by two hashes of the name |
| `script natives --unnamed <program\|folder>... [--project <file>]` | each native no list names, with its calls and the kinds of their arguments, so that a list can be written where it names most |
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
A decompile asked again, at another level or with more functions, runs on a thread of its own:
the inspector counts the functions written, and **Stop decompiling** stops it after the one it is
writing, the script keeping the source it had. A decompile another one replaces, or whose script
is closed, is stopped too. A script read from an archive and decompiled alone offers **Build
project from this archive**: on a thread of its own it reads every program of that archive and
of the archives nested in it, builds their project as `script export` does, writes it as the
names folder's `project.json` in place of the one there, and decompiles the script again with
it; the inspector counts the programs found and read, and **Stop building the project** stops it
before anything is written. A function picked in the outline while the project is being built is
not decompiled: pick it again once the build is done. The text editor shows the
source with highlighting, **Go to definition** and **Find references** follow the name under the
caret within the script, the outline lists its classes and functions, and the inspector states
the program's facts, the lists that named things, the level, and the instructions of the function
picked. **Open names folder** opens the folder the lists are read from. A body the
container reader refuses opens as the plain instruction listing where that reader takes it.
