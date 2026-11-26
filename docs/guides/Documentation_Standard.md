# Documentation standard

How OpenRage's committed documents are written, how their figures stay true,
and which checks read them. It covers `docs/`, `AGENTS.md` and every committed
README; the gitignored docs/extra/ tier of RAGE layouts is outside it.

## Kinds and headers

Every document opens with `# Title` in sentence case and one sentence saying
what it is for. One that no longer describes the tree adds one status line
under that, never a banner paragraph:

- `Status: superseded on YYYY-MM-DD by [X](X.md); kept as the record of <what>.`
- `Status: in transition to [X](X.md) until R66f.`
- `Status: snapshot of YYYY-MM-DD; [the roadmap](Roadmap.md) is current.`

| Kind | Examples | Holds | Lives in | Budget |
|---|---|---|---|---|
| Agent guide | `AGENTS.md` | Rules every session obeys, the layer map, the gates, where to read next | the repository root | 400–550 lines |
| Record | [Project_Context.md](../direction/Project_Context.md) | Dated decisions, newest first, append-only | `docs/direction/` | 250 words an entry |
| Plan | [Roadmap.md](../direction/Roadmap.md) | Per item: what, done when, where it stands, the suite that proves it | `docs/direction/` | 300 words an item |
| Specification | [OR_IO_Specification.md](../direction/OR_IO_Specification.md) | A design being built and the decisions it rests on | `docs/direction/` | 12,000 words |
| Reference | [CLI_Server_Protocol.md](../reference/CLI_Server_Protocol.md), [Plugin_ABI.md](../reference/Plugin_ABI.md) | A contract other code is written against and tests read, or the map of the tree | `docs/reference/` | the contract |
| Guide | [New_Format_Implementation_Guide.md](New_Format_Implementation_Guide.md), [Development_Workflow.md](Development_Workflow.md) | Steps, a checklist, the traps, and the policies a contributor follows | `docs/guides/` | 6,000 words |
| Feature | [Animation_Preview.md](../features/Animation_Preview.md), [Mesh_Interchange.md](../features/Mesh_Interchange.md) | How a subsystem works and what it does not do | `docs/features/` | 4,000 words |
| README | `README.md`, `plugins/README.md` | What this directory is and how to start | beside what it describes | 1,500 words |

Over budget means split, not trim: measurement logs move to the suite's doc
comment and the pull request, and a subsystem gets its own document. A plan
whose work is done takes a status line and becomes a record.

`AGENTS.md` holds rules and links only; detail goes to
[Crate_Reference.md](../reference/Crate_Reference.md), [Development_Workflow.md](Development_Workflow.md)
or a crate's `//!` header. It links plainly, never `@path`, which loads the
file into every agent session. `CLAUDE.md` is a symlink: edit `AGENTS.md`.

## Where a document lives

A document lives in the folder of its kind, per the table above.
`docs/direction/` holds the Record, Plan and Specification kinds together,
because they cite one another most, and a plan whose work is done becomes a
record there without moving. A new folder needs a row in that table, and its
name is lower case and never begins with `_`, which the documentation gates
skip. The gitignored docs/extra/ tier keeps its path, and `docs/audit/` holds the
dated audit data that a plan cites, which is data and not a document.

`docs/README.md` lists every document under its folder and owns no topic, so
its link does not satisfy the rule in the cross-references below that a
document needs a link from `AGENTS.md`'s reading list or from its topic's
owner.

A relative link is relative to the folder of the document that holds it, so a
document moved to another folder is re-linked in the same change. The status
lines above show only their shape, and their links follow the same rule.

## Writing and history

- The first sentence of a document or section is its summary. Write in the
  present tense, about the tree as it is, once, in the document that owns the
  topic; a change that makes a sentence false corrects it in the same change.
- ⚠ marks a trap: the obvious reading or action is wrong, and the sentence
  names what it costs. At most one a section; never on a status, a correction
  or history; never in a heading.
- No ✅: done is `[x]` in a checklist or the word *done*. Bold at most one
  phrase a paragraph. Headings are sentence case, unnumbered and free of
  emoji; they are anchors, so rename one only with the links into it.
- A record keeps its history: an entry is never rewritten, and one a later
  decision overturns gains `Superseded on YYYY-MM-DD by <link>.`
- Everything else states the current tree. A correction replaces the wrong
  text without announcing itself ("used to say", "measured at the time"); who
  changed what, and why, belongs to the commit and the pull request. A refuted
  reading stays only as a trap: the wrong reading, its cost, the evidence.
- A superseded document is deleted once what is still true in it has moved to
  the owner of its topic. One kept as a record keeps its body unannotated
  under its status line.
- Dates are `YYYY-MM-DD`, revisions an 8-character SHA in backticks, pull
  requests `#NNN`.

## Figures

A number in a document is a claim, and one nothing checks goes stale without
failing anything. Every figure is one of three kinds:

- **Derivable** from the tree: do not write it; name the test that holds the
  property (`the_roadmap_is_an_index_of_itself`) or the list that holds the
  members (`FORMATS` in `crates/io/or-io/src/gta5/formats.rs`), never a
  pinned total.
- **Definitional**, where the answer depends on how you count (how many
  `#[record]` declarations, how many dispatch sites): delete it.
- **Measured** over the corpus or an install, neither of which CI reaches:
  prose, with numerator, denominator, suite and revision, as in "N of M files
  (`<suite>`, at `<sha>`)". A timing names the build profile and the
  machine's load; any figure names its configuration (corpus, schema
  snapshot, or none).

A figure has one home and others link to it; an illustration uses a name, not
a number. Re-measure the row, not the number: a figure filed under the wrong
subject is accurate about something else and draws no suspicion.

`every_count_a_document_states_is_asserted_or_a_named_measurement` enforces
this on every count: a numeral or a number word from eleven up that is a
percentage or is followed by `of` or by a plural noun. Its paragraph (its item,
in the Roadmap and the Project context) names a revision, a date, a pull
request, a command or a suite that exists, or a test asserts it, or the ledger
in `src/doc_counts/figures.rs` waives it with a reason. It does not read a
figure that names a place or a version, a decimal, a quoted message, a bare
number in a table cell, or a number word below eleven, which prose uses for a
pair as often as for a count.

## Evidence

A check is evidence only if it could have failed.

- Make it fail before believing it: break what it guards (move an offset,
  withhold a declaration, feed its parser each shape it rules on, as
  `the_index_check_can_tell_the_three_shapes_apart` does) and watch it go red.
  Measure a fix by withholding it; a copy patched in compares an artefact with
  itself and agrees. A suite that goes red is a finding: never delete or
  soften it before knowing why.
- A round trip reads and writes through one description, so it cannot see a
  consistently wrong offset or a field nothing reads. Compare against
  something independent; [Oracle_Testimony.md](Oracle_Testimony.md) says what
  each kind of check can and cannot see.
- A skip is a pass to libtest: record it through the `or_test_support::skip_*`
  helpers so `skip-ledger` counts it; `just test-strict` fails on a corpus
  skip. A test reports the state it ran in; put it in the one it claims.
- Report what cannot be judged as unjudged, with the observation; never widen
  a budget to pass it. A gate that fails at random is not evidence, and a
  timing baseline is never re-captured to move a budget.

## Cross-references

One document owns each topic; the others give a sentence and a link.

| Topic | Owner |
|---|---|
| Owner decisions | [Project_Context.md](../direction/Project_Context.md); or-io's D1–D18 in [OR_IO_Specification.md](../direction/OR_IO_Specification.md) |
| Status, what is next, deferred work and known gaps | [Roadmap.md](../direction/Roadmap.md) |
| The `or-io` crate | [OR_IO_Specification.md](../direction/OR_IO_Specification.md) |
| What checks a layout once the compiled readers go | [Oracle_Testimony.md](Oracle_Testimony.md) |
| What a clone does not get | [Private_Tier.md](Private_Tier.md) |
| Crates, layers, the app's shell and their traps | [Crate_Reference.md](../reference/Crate_Reference.md) |
| Building, testing, lanes, CI, pushing and merging | the rules in [AGENTS.md](../../AGENTS.md); how and why in [Development_Workflow.md](Development_Workflow.md) |
| Adding a format | [New_Format_Implementation_Guide.md](New_Format_Implementation_Guide.md) |
| Figures, evidence and the documentation gates | this document |
| Which folder a kind lives in | this document |
| RAGE binary layouts | the gitignored docs/extra/ tier |

Link documents with relative Markdown links and cite code by backticked
repo-relative path; nothing checks link targets or anchors, so follow a link
before committing it. Link a docs/extra/ file, never backtick its path, and
keep citations of the leaked source by path, file or line in that tier. A
document neither `AGENTS.md`'s reading list nor its topic's owner links to is
folded into that owner or deleted.

## Vocabulary

- **Gen8 / Gen9** name a resource's target, read from its own RSC7 version;
  **Legacy / Enhanced** name an edition or an install, and any text quoting
  what a user sees: "a Gen9 `.ytd` from an Enhanced install". A phrase a gate
  pins keeps its wording.
- Types, fields and enums go by their declared names, the engine's since D15;
  crates and modules by today's paths; "target", "title" and "container" as
  [Project_Context.md](../direction/Project_Context.md) defines them. WS2 or Phase 5 is link
  text, not a name for the thing.

## Gates that read documentation

A documentation-only pull request still runs the formatting, provenance and
encoding lints, `cargo test -p openrage` (every `src/` gate) and the
toolchain, CLI and plugin suites below (`.github/workflows/ci.yml`); run the
same before pushing.

| Gate | Reads | Fails when |
|---|---|---|
| `src/doc_citations.rs` | every `.md` outside `external/`, docs/extra/ and symlinks | a backticked path under `src/`, `crates/`, `docs/`, `tools/`, `scripts/`, `assets/`, `plugins/`, `.github/` or `external/` does not exist; a backticked `snake_case` name with two or more underscores appears in no Rust, TOML, PowerShell, shell, YAML or WGSL source outside a comment, nor as a `.rs` file's name; an `ELSEWHERE` or `IN_A_COMMENT_ONLY` permit resolves or is no longer cited; the whole set cites too few paths or names to be a working parse |
| `src/doc_counts.rs` | `docs/direction/Roadmap.md`, [Crate_Reference.md](../reference/Crate_Reference.md) | the Roadmap's checkboxes and `Rnn` headings are not one set (a decomposed number keeps its heading while a lettered part is scheduled); a `FormatCapabilities` field is not named in backticks |
| `src/doc_counts/figures.rs` | every `.md` in `docs/` and the crates, `README.md`, `AGENTS.md` | a count is asserted by no test, anchored in its paragraph (its item, for the Roadmap and the Project context) to no revision, date, pull request, command or suite that exists, and waived by no ledger entry; a ledger entry excuses nothing; a document's figure and the code's constant it states differ |
| `src/doc_comments.rs` | Rust doc comments | one carries `# Features`, `# Usage`, `# Arguments`, `# Panics` or `# Returns` |
| `crates/foundation/or-test-support/tests/toolchain_pin.rs` | `AGENTS.md`, the workflows | `AGENTS.md` does not name the channel `rust-toolchain.toml` pins, or a job builds on another |
| `crates/tools/or-cli/tests/the_external_surface_is_what_it_documents.rs` | `docs/reference/CLI_Server_Protocol.md` | a served route is not named, a `"cmd":` example names a route the server lacks, or a route is not a tool of `crates/tools/or-cli/src/mcp.rs` |
| `crates/domain/or-project/tests/the_acceptance_sheet_is_what_its_guide_and_the_roadmap_say.rs` | `docs/guides/In_Game_Acceptance.md`, `docs/direction/Roadmap.md` | the guide's table of lines is not the lines of `crates/domain/or-project/src/acceptance.json`, word for word and in order, a line names a row with no heading in the Roadmap, or the guide's example of a result is not one the sheet reads |
| `crates/tools/or-cli/tests/a_built_folder_is_read_against_the_acceptance_sheet.rs` | `docs/guides/In_Game_Acceptance.md` | an `openrage-cli project` command of the guide's recipe does not run over a synthetic map and type file, the recipe lacks one of `create`, `import`, `build` and `acceptance`, or the build they make is not the one the last reads |
| `crates/domain/or-plugin/tests/how_anything_crosses_the_boundary.rs` | `docs/reference/Plugin_ABI.md` | it lacks "ABI version N" for the live `ABI_VERSION`, the oldest version beside it, "an older plugin still loads", the version that added a newer capability, any status's key, or a host function, a capability key or an export a session calls |
| `scripts/check_encoding.ps1`, from the pre-commit hook, `just lint-encoding` and hosted CI | every `.md`, `.rs`, `.toml`, `.ps1` and `.yml` | a UTF-8 BOM, or mojibake from a round trip through cp1252 |
| `scripts/lint_provenance.ps1`, run the same way | every tracked file | committed game content; a leaked-source path or file-and-line citation, a document included |

A gate that reads a document fails, never skips, when its claim is missing;
asserts its anchor matches exactly one line; tests its extractor on sample
text; requires the declarations it compares, since a job on `@stable` declares
nothing; finds another tree with `or_test_support::openrage_root`; and joins
the documentation-only steps of `.github/workflows/ci.yml` and this table.

⚠ An odd number of backticks outside a fence blinds `src/doc_citations.rs` to
the rest of that file. It splits the text on single backticks and skips a
token that spans a line or holds a space, wildcard or bracket, so an inline
triple backtick leaves every later citation unchecked while the gate passes.

`src/shell_budget.rs`, `src/suppressions.rs`, `src/state/tests.rs` and
`the_shell_is_flat_rather_than_deep` hold what [Crate_Reference.md](../reference/Crate_Reference.md)
argues about the shell (a code-line budget per file in `src/`, a reason beside
every suppression that hides unused code or covers a module, a flat `AppState`
with no panel state) but never read
its text, so they cannot catch a stale sentence.
