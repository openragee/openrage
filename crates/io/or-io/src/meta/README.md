# Schema snapshot

What the metadata schema snapshot is, where the editor and the CLI find one and what a run without one loses; no snapshot is ever committed.

`schema_snapshot.bin` is an index of GTA V's metadata schema: which structs
exist, their fields, and what those are called. It is what turns a field's
hash back into its name in the inspector, and the editor and the CLI install
the first one they find. It is produced by parsing a PSC corpus, which makes it a
machine-extracted derivative of a source tree that is not ours to
redistribute, so it stays on the machine that made it.

## Without one

- Binary PSO, RBM and RBF0 still read, write and round-trip: PSO and RBF0
  byte for byte, RBM on its decompressed payload.
- A ped variation or creature record built from nothing saves too, against the ped
  family's types declared in code (`typed::ped_types`) and held to the tables the staged
  documents state; a record of any other family built from nothing is refused.
- Field names in `binary::public_names`, which covers the world metadata
  family, still resolve. Over the 6,071 documents of the metadata corpus a
  snapshot-less build names 122 of the 9,629 distinct name-hashes they carry
  (measured for #144 at `6719fdfc`, and recorded in
  `crates/io/or-io/src/meta/binary/public_names.rs`).
- Every other name, whether a field, a type, an enum value or a hashed
  string, is spelled `hash_XXXXXXXX` (eight upper-case hex digits) in the
  editor and in exported XML, and an import reads `hash_` followed by exactly
  eight hex digits back to that hash; `hash_table` stays a name
  (`binary::hash_index::spelled_hash`).
- PSO and RBM XML import takes each struct's layout from type tables
  (`binary::type_tables`): the donor's (`meta from-xml --like`, and the
  document itself in `canonical::converge`), else those of every metadata
  file in the install, which `meta from-xml --as` reads once an import fails
  for want of a definition. A structure no table states is refused by name.
  With `OPENRAGE_SCHEMA=none`, all 3,527 PSO, 2,338 RBM and 206 RBF documents
  of the corpus reach their fixed point on their own type tables
  (`crates/io/or-io-verify/tests/meta/xml_converges_without_a_snapshot.rs`,
  measured for R37 over `2dfd9dcf`). Where a snapshot declares a type, its
  definition wins.
- XML whose root or pointer type the library lacks opens as the tree its
  elements state (`instance::parse_schemaless`): an `Item` list is an array,
  a `type=` element a pointer, a leaf text, a `value` or a vector, and an
  element the parser's dialect could not have written is refused by name.
  It saves back as XML. A binary `.srl` needs no schema at all (`srl`).
- A caller that has to make the same XML and the same bytes with a snapshot
  installed and without, as a project's metadata source does, wraps its reads
  and writes in `binary::hash_index::with_public_names_only`, which names every
  hash on that thread from the public table alone.
- An enum name the build cannot resolve and a value no variant lists are
  spelled differently, so a save keeps the value
  (`crates/io/or-io-verify/tests/meta/enum_variant_sentinels.rs`).

`or_io::meta::has_default_library()` says whether one was installed, so the UI
can explain itself.

## Where the editor and the CLI find one

Nothing compiles a snapshot in. At run time `or_title::schema` takes the first
of these that exists, and the host installs it as `or_io::meta`'s default library
(OD-SCHEMA-LOC, [docs/direction/Project_Context.md](../../../../../docs/direction/Project_Context.md)):

| Path | |
|---|---|
| `$OPENRAGE_SCHEMA` | an explicit file |
| `schema_snapshot.bin` in the user's configuration directory | beside the keys cache |
| `openrage-corpus/or-meta/v3/schema_snapshot.bin` | the shared store, only from a source tree, found by walking every ancestor of it, so one snapshot serves every worktree and lane; a lane's parent is `openrage-lanes`, so a parent-only lookup would find nothing |

`or_io::meta::default_origin()` reports which file was installed, and
`just snapshot-status` prints what this tree finds with its contents and age.
A suite finds one through `or_test_support::schema_snapshot`, which reads the
variable and then the store, never the configuration directory, and records a
skip when a suite that needs names has none.

`OPENRAGE_SCHEMA=none` (or `off`) runs as a clean clone does. It is a word
rather than an empty value because PowerShell deletes a variable set to `""`.
`just test-no-pack` sets it along with the other clean-checkout variables
([docs/guides/Development_Workflow.md](../../../../../docs/guides/Development_Workflow.md));
the metadata suites still find their own corpus there, so that run tests a corpus with no
snapshot.

## A second title's snapshot

GTA VI has a snapshot of its own, of the same format, which `openrage-cli meta snapshot --par-tree <dump.json>` writes from a parser dump (`par_tree`). It is found the same three ways under its own names (`OPENRAGE_GTA6_SCHEMA`, `gta6_schema_snapshot.bin` in the configuration directory, `or-meta/v3/gta6_schema_snapshot.bin` in the store), and `OPENRAGE_SCHEMA=none` turns it off with the first. It is installed for its names alone (`install_names_snapshot`), and no structure of it is laid out, since the two titles declare structures of one name differently and a GTA VI body states its own layout. Its names are given only while a reader or writer of GTA VI's metadata holds `binary::hash_index::title_names()`, first among the index's sources; anywhere else the index answers as if it were not installed, so a GTA V document never shows a name of GTA VI's.

## Producing one

```bash
just snapshot <path to a PSC tree>
```

This is `openrage-cli meta snapshot --psc-root <dir>`, which writes the shared
store above the source tree; `--out <file>` or `--store <corpus-dir>` writes
elsewhere, and `OPENRAGE_PSC_ROOT` stands in for `--psc-root`.

A snapshot is `bincode` of the type model, so a tree with another model cannot
decode it, and installing one fails with an error. The store therefore keeps
one directory per serialised shape, named by `SNAPSHOT_FORMAT` (`v3` today);
the first shape predates the directories and sits at the store's root for
older trees. When `the_snapshot_format_names_every_shape` stops compiling,
list the new variant or field there and raise `SNAPSHOT_FORMAT` with
`or_title::schema::STORE_FILE`.

`openrage-cli meta snapshot` also writes `schema_snapshot.stamp` beside the
snapshot: an FNV-1a digest of `crates/io/or-io/src/meta/schema`, the sources
that decide what a snapshot holds. From a source tree, installing a snapshot
recomputes the digest and warns when it is stale or `UNSTAMPED`, and uses it
anyway; `or_io::meta::snapshot_age` reports the same.

Where the library and `binary::public_names` spell a hash differently, the
library wins: `HashIndex::with_public_names` seeds after it, and the first
insertion stands.

## A snapshot that could ship

Many of the names are public: CodeWalker's `MetaNames.cs` publishes a large
hash-to-name table, though its repository carries no licence file
([LICENSE-third-party.md](../../../../../LICENSE-third-party.md)), and retail files
store many of their own strings. Whether to build a redistributable snapshot
from such sources is an owner and lawyer decision, not open work, and D5
decides that `or_io::meta` bundles no schema and
takes one as data
([docs/direction/OR_IO_Specification.md](../../../../../docs/direction/OR_IO_Specification.md#metadata)).
