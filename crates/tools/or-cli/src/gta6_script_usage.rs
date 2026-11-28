//! What `script --help` prints, for every game the command reads.

use or_io::gta6::script::Level;

use super::lists::{LABELS, NAMES_FOLDER};

/// The usage of `script`, for every game it reads.
pub fn usage() -> String {
    let levels = Level::ALL.map(Level::name).join(", ");
    format!(
        "\
Inspect, disassemble or decompile a compiled script: a GTA V .ysc, or a GTA VI program.

Usage: openrage-cli script <src> [--natives|--strings|--statics|--functions] [--json]
                                 [--offset <n>] [--limit <n>]
       openrage-cli script disasm <src> [--function <f>]... [--class <c>]... [--no-bytes]
                                        [--project <file>|auto] [-o <file>]
       openrage-cli script decompile <src> [--function <f>]... [--class <c>]...
                                           [--with-classes]
                                           [--level <name>] [--no <pass>]...
                                           [--project <file>|auto | --alone]
                                           [--labels <file>]... [-o <file>]
       openrage-cli script check <file-or-folder>...
       openrage-cli script list <archive.rpf> [inner/path] [--json]
       openrage-cli script export <build-folder|archive.rpf|folder> [-o <folder>]
                                  [--level <name>] [--no <pass>]... [--threads <n>]
                                  [--labels <file>]... [--summary]
                                  [--changed-since <folder>]
       openrage-cli script passes
       openrage-cli script names-search <program|folder>... [--words <file>]... [-o <file>]
       openrage-cli script natives --unnamed <program|folder>... [--project <file>]
       openrage-cli script download-natives
       openrage-cli script lifted <listing.ysc.cs> [--level <name>] [--no <pass>]... [-o <file>]
       openrage-cli script lifted --census <folder> [-o <folder>]
       openrage-cli script compiled [--natives] [--listing] <program>...

  <src> is a file, or a GTA VI archive and one entry of it:
          <file.ysc>                               a GTA V script
          <file>                                   a deciphered GTA VI program, told by its
                                                   first bytes: it has no extension
          <archive.rpf> [inner/path] --entry <e>   a GTA VI program in an archive (RPF9)
        <e> is the entry's name, its 0x... or hash_... hash, or its index, as `script list`
        prints them; a name the table does not hold is hashed and matched.

  What is read of a GTA VI program: its container (functions, types with their fields and
  method records, imports, the functions the engine calls by name) and its bytecode. What is
  not: the hash or the name of a native, since the code calls a native by the address of its
  handler and only supplied lists say what is there; initial values of globals and statics,
  since the container holds none. Nothing of a program is ever run.

  disasm      An instruction listing: jump and switch targets resolved, calls named by
              function, and a native by what the lists call its handler.
  decompile   Source: functions, expressions, if/else, loops and switches, with a labelled
              goto only where none fits; for a GTA VI program also the classes and the source
              constructs its compiler's expansions prove, each by a pass `script passes`
              names. A GTA VI program decompiled with no project is decompiled alone: a
              function that needs what another program states is written as a comment
              saying so, and stderr says how many were. With no --project, the project.json
              beside the program is read, else the one of the `{NAMES_FOLDER}` folder, else
              one is built as --project auto builds it and kept in that folder, which the
              editor reads too: the first decompile takes minutes, the next ones read it.
              No project ships, since one holds for one build of the game. --alone
              decompiles with none, lightly and faster.
  check       Decode each script and check it against its own file, each by the reader its
              first bytes ask for. A folder stands for every file under it that starts as a
              GTA VI program does. Exits non-zero if any script does not decode.
  list        The GTA VI programs an archive holds: index, entry hash, size, program hash,
              entry name. Every enciphered entry is deciphered to tell, so a large archive
              takes a while.
  export      Decompile every GTA VI program of a build folder, of an archive or of a folder
              of deciphered programs, and print one line of counts per program and a total.
              The constructs counted are those the passes read, whether or not the printer
              then writes one otherwise than the code it was read from.
              With -o <folder> it writes there project.json (what the programs state of
              each other: types, fields, signatures, calls), one .cs per program written
              with it, scripts.json (what was read, where each program is, with which
              lists, level and passes), and index.md, a table for a person to read: each
              source with its program, its name where a list gives one, its functions and
              classes, and what was not lifted. A program that fails is reported and the
              export goes on; it exits non-zero if any did. --summary prints, in place of
              a line a program, the programs that failed and the totals as a table.
              --changed-since <folder> compares each source, as text, with the one the
              export in that folder wrote of its program, and writes only those that differ:
              it prints a line for each source whose code changed, that is new, or that the
              folder holds and this export writes no more, then how many compare each way; a
              source that differs only in its first lines of comments is counted, not named.
  passes      The passes and the level each belongs to, one per line, as --no and --level
              take them. A name no pass reads yet is marked `rebuilds nothing yet`.
  names-search  A names list of what a word search proves of the programs' types, fields
              and programs, each row by two hashes of its name; `script names-search` with
              no path prints all it takes.
  natives --unnamed  Each native no list names, with its calls and how they use its
              arguments, so that a list can be written where it names the most calls.
  lifted      Readable source from a lifted listing of a GTA VI script, the text another
              decompiler writes from its bytecode; `script lifted --help` says what it takes.
  compiled    A GTA VI program's header and a plain listing of its instructions, with each
              native by its address; `script compiled --help` says what it takes. It lifts
              nothing: `disasm` and `decompile` are the readers to use for source.
  --natives   The natives the script calls: by hash for a GTA V script, named where a list
              does; by handler address for a GTA VI program, with the cells each takes and
              returns, and its hash and name where the lists give them.
  --strings   The script's strings, one per line; for a GTA VI program the strings its code
              pushes, each under its offset in the pooled data.
  --statics   A GTA V script's initial static values; the leading ones are arguments. A
              GTA VI program's container holds none.
  --functions A GTA VI program's functions, one per line: offset, bytes, cells in and out,
              name. --offset <n> leaves the first n out and --limit <n> prints n at most,
              and stderr says where the page stands. With --json, one function a line.
  --function <f>   Only this function: 0x<offset>, func_<offset>, the offset as a listing
                   prints it, its name as --functions prints it, or the name a list gives
                   the method it is, alone or after its class's (Class.Method). Repeat for
                   several. A source is then written without the types that hold no code of
                   it, and says how many; --with-classes writes them too. (GTA VI)
  --class <c>      The methods of this class too: its name as a list gives it, or the
                   type_ hash a source writes. Repeat for several. (GTA VI)
  --no-bytes       Leave the column of bytes out of a GTA V listing; a GTA VI listing has
                   none.
  --level <name>   How far the source is taken from the bytecode: {levels}.
                   Default: source. (GTA VI)
  --no <pass>      Leave one pass off, by the name `script passes` gives it; the code stays
                   in the lower form. Repeat for several. (GTA VI)
  --project <file> The project.json an export wrote: what the other programs state of the
                   types, fields and functions this one uses. (GTA VI)
  --project auto   Build that project in memory first, as export builds it, of every program
                   of the folder the program file lies in, or of the archive (and the archives
                   nested in it) the --entry is read from; stderr says how far it is. It reads
                   at most 4 GiB, and --threads <n> sets its threads. A file called `auto` is
                   named as ./auto. (GTA VI)
  --alone          Decompile with no project: none is read and none is built, so a function
                   that needs another program's has no body, and the decompile is faster.
                   (GTA VI)
  --threads <n>    The threads an export, or --project auto, works on at most. Default: 8.
  --json      The same facts as JSON, for `script <src>` and `list`. (GTA VI)
  -o <path>   Write to a file (a folder for export) instead of printing. (GTA VI)

Names:
  --natives-db <path>   A natives list, JSON or `0xHASH NAME` text. Repeat for one list per
                        build; each script takes the list naming most of its natives. A
                        list that is named must read. Without the flag, OPENRAGE_NATIVES is
                        read as ;-separated paths, and without either, what download-natives
                        fetched. A natives list names by hash: it names a native of a GTA VI
                        program only where a names list gives the hash of the handler at the
                        address called, and a source's first lines say so of a list that
                        could name nothing.
  --names <path>        Names of a GTA VI program's types, programs, methods and fields: one
                        `0xHASH name` per line, or a tab-separated table. A table whose
                        first columns are `hash` and `rva` gives each native's hash by
                        the address of its handler. A file of one word a line with no
                        hash, or a vocabulary file OpenRage wrote, gives candidates: a
                        word names a type, a field or a module only where its own hash
                        is the one the program holds, and a method only while the words
                        read number 65,536 at the most. Repeat for several. Without the flag,
                        the `{NAMES_FOLDER}` folder of the configuration directory is
                        read by one rule: a file whose name starts with `natives` is a
                        natives list where it reads as one, else a names list; any other
                        file is a names list where it reads as one, else a natives list. A
                        file that reads as neither is said on stderr and passed over. A
                        list every row of which proves itself by its hash is read before
                        the others, so a label given on trust never displaces a proof. A
                        file whose name holds `candidate` holds guesses and is passed over:
                        only --names reads it. A source's first lines name each list read
                        with how much it named that no list before it did.
  --labels <path>       A list of guesses (labels) at what natives do: one `0xHASH _LABEL` a
                        line, `NS::_LABEL` allowed, or a table whose columns name `hash` and
                        `label`. It is never read unless named: a file of the
                        `{NAMES_FOLDER}` folder whose name holds `{LABELS}` is passed over and
                        said. Each label opens with `_` and is one name, or the file is
                        refused. A label is written for a native only where no natives list
                        names it, with no parameter or result; it names no variable and never
                        counts as a name. Repeat for several; a later file's label stands in
                        place of an earlier one's. A source's first lines name each labels
                        file read and how many natives it labelled. (GTA VI)
  download-natives      Fetch alloc8or's natives lists and CSY0N's crossmap from GitHub into
                        the configuration directory, recording each file's commit. Runs only
                        when you ask. They name GTA V's natives and none of GTA VI's.
  ⚠ A package ships GTA VI lists in a `{NAMES_FOLDER}` folder beside the executable, read
    before the configuration directory's, where a file of the same name stands in place of
    the shipped one. A hash no list names stays a hash. The
    first lines of a source say which lists named things, and the last lines of
    `script <src>` do; a listing and a list of functions or natives use the lists and do
    not say which.

GTA VI archives:
  --keys <snapshot>     The key snapshot, as `rpf9` takes it; without the flag,
                        OPENRAGE_GTA6_KEYS or the snapshot beside the keys cache.
  --exe <file>          The executable a masked snapshot opens with; without the flag,
                        OPENRAGE_GTA6_EXE.
  Entry names come from OPENRAGE_GTA6_NAMES and the names the app harvested; without them
  an entry is named by its hash.
"
    )
}
