# AGENTS.md — OpenRage

The rules every agent session in this repository follows, and where to read
next.

OpenRage is an asset editor for Rockstar's RAGE engine: it reads, edits and
writes the files GTA V Legacy and Enhanced ship. It is Rust 2021 on the
toolchain `rust-toolchain.toml` pins, nightly-2026-10-08 (the dev and test
profiles build with Cranelift and `-Zthreads`, which only nightly has), with an
egui + winit + wgpu desktop app, Windows first. Every session loads this file, so it holds rules
and links. `CLAUDE.md` is a symlink to it: edit `AGENTS.md`.

## Read before you start

| When you are… | Read |
|---|---|
| Deciding direction, or about to reopen a decision | [OR_IO_Specification.md](docs/direction/OR_IO_Specification.md) (D1–D18) and [Project_Context.md](docs/direction/Project_Context.md) |
| Picking work, or asking what is done | [Roadmap.md](docs/direction/Roadmap.md) (R66 is the current plan) |
| Declaring a GTA V structure in `or-io` | [OR_IO_Specification.md](docs/direction/OR_IO_Specification.md): [declaring a structure](docs/direction/OR_IO_Specification.md#declaring-a-structure) and [the target](docs/direction/OR_IO_Specification.md#the-target-d16) |
| Bringing up a format end to end (recon, CLI, app) | [New_Format_Implementation_Guide.md](docs/guides/New_Format_Implementation_Guide.md) |
| Working in a lane, testing, pushing, reading CI | [Development_Workflow.md](docs/guides/Development_Workflow.md) |
| Looking for a crate, a module or the app's structure | [Crate_Reference.md](docs/reference/Crate_Reference.md) |
| Writing or changing a document | [Documentation_Standard.md](docs/guides/Documentation_Standard.md) |
| Mapping a C++ RAGE struct onto file bytes | the local [docs/extra/README.md](docs/extra/README.md) and its specifications |

[Other documents](#other-documents) lists the rest.

## Working principles

Four habits, adapted from
[andrej-karpathy-skills](https://github.com/multica-ai/andrej-karpathy-skills),
that the rest of this file applies. They trade speed for care; on a trivial
change, use judgement.

- **Think before coding.** State your assumptions. When a request has two
  readings, say which you took or ask; when something is unclear, stop and
  name it. A dated owner decision is not reopened in code: read it first, and
  if the tree contradicts it, report that.
- **Simplicity first.** Write the least code that does what was asked: no
  speculative feature, no abstraction for one caller, no option nobody asked
  for. A malformed file is not an impossible case, so a reader still refuses
  bad bytes with an error.
- **Surgical changes.** Every changed line traces to the task. Match the
  surrounding style, leave unrelated code, comments and formatting alone and
  mention what you noticed instead, and remove only what your own change left
  unused. Other agents work in other lanes at the same time.
- **Goal-driven execution.** Turn the task into a check that can fail: a
  test that reproduces the bug before the fix, a suite named before the claim.
  For multi-step work, state the plan with the check each step ends on, and
  see it fail once before trusting it ([Evidence](#evidence)).

## Direction: the or-io crate

GTA V's structures are declared in public in `or-io` (with `or-io-macros`),
and `or-asset` and `or-meta` merged into it and were deleted (D5, D10). The
plan is roadmap R66, whose open rows are the history rewrite and
publication (R66g, R66h).

- **Where the tree stands.** `crates/io/or-io/src/gta5/` declares every format
  `crates/io/or-io/src/gta5/formats.rs` lists; the RSC7 header is declared in
  `or_io::container::resource`. The app opens and saves every paged format of
  both targets as an owned value with no pack, and so does `or-cli`.
  `or-asset` and its compiled readers are deleted (R66f): the recorded
  testimony and the behaviour baselines are the checks that outlived them.
- **New format work goes to or-io**, declared and opened as an owned value.
- **D15: a declaration follows the GTA source.** Type, field and enum names
  are the engine's, in Rust casing (`GrcTexture`, `vertices_count`).
  `#[record]` computes offsets from `repr(C)` field order; padding is a
  declared field or `#[pad_after(N)]`; `#[at]` is an optional assertion; there
  is no `#[provenance]` attribute. CodeWalker, the corpus and the testimony
  cross-check the layout. An owned declaration refuses an `unknown*` or `unk*`
  field (`decl::refusals`), and `#[record(raw, ...)]` is retired and
  refused.
- **D16: a file opens as an owned value.** `T::open`, field reads and
  assignments, `Vec` edits and `save()`, with errors only at open and save.
  Build toward [the target](docs/direction/OR_IO_Specification.md#the-target-d16) and
  migrate each format once, to that shape.
- **When a format counts as finished.** A format is declared once `FORMATS`
  lists its `#[record]` roots, and finished when
  [the gates per format](docs/direction/OR_IO_Specification.md#the-gates-per-format)
  hold for every target it ships: typed reads, the re-keyed testimony, the
  canonical fixed point (and `owned_saves_as_tree` for an owned format), the
  alignment census and D15's names; the structural diff against the private
  pack is retired for every format (D17). The shared gate code is
  `crates/io/or-io-verify/src/gates.rs`, which `declared_gates!` writes as a
  format's tests.
- An owned value carries what the file states and its fields do not, such as
  a counted array's capacity, in its hidden `__carry`, because that capacity
  is an allocation the game sized.
- **Byte-identity with shipped files is not the goal.** A save writes
  OpenRage's deterministic canonical form, and saving that again is
  byte-identical.
- **A file's target is its RSC7 version** (`Format::target_of_version`),
  never the install it came from.
- **Scope.** Every RSC7 format, the RSC7 header, RPF7's headers and entry
  table, and GXT2 are declared in `or-io`; `.ysc` stays ordinary code until a
  corpus exists (OD-YSC). DDS, FXC, AWC, `.rel` and `.gfx` are ordinary code
  behind features, and so is every other archive generation (RPF0, RPF2,
  RPF3, RPF4, RPF6, RPF8 and IMG1 to IMG3; `RpfVersion`,
  `container/rpf/layout/`): another title's tables are not declarations of
  the GTA source.
- **Never commit** a citation of the GTA source tree by path, file or line, or
  a comment that says code came from it; notes about it go in `docs/extra/`.
  Code ported from it is allowed (owner, 2026-10-08), and its comments say what
  the code does and never where it came from. The provenance lint, which the
  pre-commit hook runs, fails on a citation in any tracked file, and on
  committed game content.
- **Keys.** The AES key, the hash table and the AWC key are extracted from
  the user's own executable. NG keys come from a key snapshot: the user's own
  (`magic.dat` beside the keys cache for the app and CLI,
  `OPENRAGE_KEY_SNAPSHOT` for the library's default path), else the one
  `or-io` bundles behind its default `bundled-keys` feature (D18), which
  decodes only with the AES key from the user's executable. The tree
  holds that snapshot once (`the_key_snapshot_is_in_the_tree_once`), so no
  install-gated suite needs `OPENRAGE_KEY_SNAPSHOT`. RDR2's cipher tables are
  a second snapshot of the same kind, `rdr2_magic.dat` (owner, 2026-10-08),
  unlocked by the AES key in `RDR2.exe` and held to one copy by the same test.
- **The testimony** stays in `openrage-corpus/testimony` by the owner's
  exception: do not move it or repoint `the_recorded_testimony_holds`; the
  suite itself may move into `or-io-verify` under that name (OD-TESTIMONY).
  The exception also covers the testimony's frozen offset key, taken once
  from the version 12 pack and never retaken (OD-TESTIMONY,
  OD-TESTIMONY-KEY-FROZEN), and R66e's behaviour baselines,
  digests only, in openrage-corpus/baselines, written only by
  `just baselines-record` (OD-BASELINES).
  `openrage-corpus` otherwise holds game files and the metadata schema
  snapshot store, which stays there by the owner's ruling of 2026-09-26
  (OD-SCHEMA-LOC).

## Workspace and where code goes

| Directory | Crates |
|---|---|
| `crates/foundation/` | or-job, or-test-support, or-transport |
| `crates/io/` | or-io, or-io-macros, or-io-verify, or-kraken, or-media |
| `crates/title/` | or-title |
| `crates/domain/` | or-addonped, or-content, or-depends, or-editor, or-interchange, or-maps, or-particle, or-particles, or-plugin, or-project, or-source, or-weapons, or-world |
| `crates/runtime/` | or-anim, or-renderer, or-simulation |
| `crates/tools/` | or-cli (`openrage-cli`), or-shader-front, or-shader-tool, xtask |
| `src/` | the `openrage` app, which also builds the `openrage-cli` binary |

- **Dependencies run one way:** foundation → io → title → domain → runtime →
  app. The app depends on every layer and on `or-cli` and `or-shader-tool`.
  No edge points up. `xtask`, the repository's own tasks, is run and not
  depended on, and it depends on no workspace crate.
- A crate's directory names its layer, and each crate imports the layer it
  needs directly. There are no facade crates, so a wrong-way edge shows in the
  diff rather than only in `cargo tree`.
- **Placement:** code that needs to know where a file lives goes in
  `or-title`; code that needs only the bytes goes in `or-io`, but a new decoder
  of a file's audio or video goes in `or-media`, where code ported from an open
  implementation keeps its licence out of `or-io`.
- Edges that stay absent: `or-anim` → `or-particle` (the playback clock is
  `or-transport`).
- The app owns egui integration; `or-renderer` has no egui dependency.
- Adding a title is one `GameProfile` impl and one registry line
  (`crates/title/or-title/tests/adding_a_title.rs`); no pack is loaded for it
  (OD-OTHER-TITLES). Registry order matters:
  an Enhanced install also ships `GTA5.exe`.

The full tree, per-crate notes and traps, and the resource machinery are in
[Crate_Reference.md](docs/reference/Crate_Reference.md).

## Lanes and the shared desktop

Every agent works in its own git worktree, a lane, on the owner's desktop.
[Development_Workflow.md](docs/guides/Development_Workflow.md) has the janitor, the
agent limits and the disk budget in full.

- **Make a lane with `just agent-worktree NAME`, from the checkout** (from a
  lane it resolves the lane root one level too deep): `../openrage-lanes/NAME`
  on branch `agent/NAME`, with a `.agent-worktree` marker. The janitor reports
  a worktree made anywhere else.
- **Never create a file or directory outside the lane root**, and never
  inside a sibling project in the organisation's folder. The exceptions are
  what a recipe writes on purpose (`just snapshot` and
  `just baselines-record` into `openrage-corpus`) and scratch under
  `$env:TEMP`.
- Leave `CARGO_TARGET_DIR` and `CARGO_INCREMENTAL` unset. A lane builds into
  `target/` inside the worktree, incrementally, with sccache for the registry
  crates where installed (the justfile sets only `RUSTC_WRAPPER`); outside one
  an ambient value passes through, which the self-hosted runner relies on.
- **After a merge:** `just janitor-dry`, then `just janitor`, which needs an
  authenticated `gh` and runs on every OS
  ([the janitor](docs/guides/Development_Workflow.md#cleaning-up-the-janitor)).
  It removes a worktree only
  when its work has landed (a merged pull request whose head is its HEAD, or a
  closed one whose `(#N)` subject is on `main`), it is clean, it holds no
  ignored file the checkout lacks, nothing has written to it for an hour and
  git has not locked it. It never deletes a branch.
- **Delete a landed branch on origin and locally.** The merge's
  `--delete-branch` removes the remote one (otherwise
  `git push origin --delete <branch>`); remove the local one once its worktree
  is gone with `git branch -D`, because `-d` refuses a squash-merged branch.
  Never delete a branch whose work has not landed on `main`.
- **Copy a `docs/extra/` file written in a lane into the checkout's
  `docs/extra/` before calling the lane done.** It is gitignored, in no
  commit, and dies with the worktree; the janitor holds such a lane but cannot
  save the file.
- ⚠ `git stash push` on a tree whose only changes are untracked stashes
  nothing, and the `pop` paired with it takes another session's entry. Read
  `git status` before stashing and `git stash list` after, or commit a WIP
  instead.
- **Agent limits are off unless the owner runs `just limits-start`**, and a
  restart turns them off again. While on, agent processes run at Idle
  priority under memory ceilings and the saved CPU cap, and one past a
  ceiling is killed. `just limits-status` says whether they are on.
- Scope an install census with `--ext`, keep the default sample, and never
  detach one you will not reap. `openrage-cli formats` stops classifying at a
  byte budget even with `--all`; `--no-byte-budget` or
  `--budget-gib unlimited` removes the only memory ceiling it has.

## Never open a window

`just dev`, `cargo run -p openrage --bin openrage` and anything else that
starts the editor take the focus of the owner's desktop. Do not run them.

- Use `just self-check`, which is
  `cargo run -p openrage --bin openrage -- --self-check`. It builds the real
  `AppState` and draws every panel, every workspace, the start page and the
  settings window through real egui frames, judged against a blank control
  panel, and exits non-zero on a panic, a panel that painted nothing, or a
  run that wrote no preferences.
- It resolves its configuration to a scratch directory it removes, and
  `src/self_check.rs` refuses to run against the real one; set
  `OPENRAGE_CONFIG_DIR` to keep what it writes.
- `--viewport` adds a surfaceless wgpu device and draws through the render
  bridge. Nothing reaches winit or a window surface, so if a change cannot be
  verified without a window, say so and hand it to the owner.
- **To see what a change draws, use `just shots SCRIPT DIR`**: the same frames
  driven by a script of steps (open a file, click, type, `shot NAME`), saved as
  PNGs. Look at before and after screens of a change to what the app draws,
  and say in the pull request what they show
  ([screenshots](docs/guides/Development_Workflow.md#screenshots)).

## Build and test

Prerequisites: `just`, and `cargo install cargo-nextest --locked`. The
nextest recipes refuse to start without it (`scripts/require_nextest.ps1`)
rather than fall back to a different selection. `just --list` shows every
recipe.

| Recipe | What it runs |
|---|---|
| `just ready` | the checks before a merge, over the packages the change touches against `origin/main` and the packages that depend on them: clippy, and the default test tier with each test binary run once |
| `just ready-full` | the same over every package, whatever changed |
| `just test-affected [PART…]` | `ready` without clippy, kept to the tests whose names hold a PART: the loop while editing |
| `just test-crate X` | one crate's library tests of the default tier, a process per test |
| `just test-format MODULE` | one format's or-io-verify corpus suite, slow tier included |
| `just test-lite` | libraries, binaries and doctests; no `tests/` binary |
| `just test` | every test of the workspace under nextest, the slow tier included, doctests, then the skip ledger: about twenty minutes |
| `just slow-tier` | rewrite the slow tier in `.config/nextest.toml` from the timings of the last `just test` |
| `just test-no-pack` | the same as a clean checkout sees it |
| `just test-strict` | fail instead of skipping, for a machine with a corpus staged |
| `just lint` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `just fmt` / `just fmt-fix` | rustfmt over every target, in bounded chunks |
| `just snapshot PSC_ROOT` | build the metadata schema snapshot into the shared store |

- **Test the least that covers the change (owner, 2026-11-20).** A default
  run is held to about 30 s, so the tests are two tiers: the default tier,
  which every gate and a bare `cargo nextest run` take, and the slow tier,
  which `.config/nextest.toml` names by module (the corpus walks, the install
  suites, the plugin guests) and only `--profile all` runs. `just test` is
  every test, and is run on `main` between waves, never as a lane's gate.
- **A tier that did not run has not passed.** Before claiming a format's read
  or write path works, run that format's slow suite: `just test-format MODULE`,
  or `cargo nextest run --profile all -p CRATE`. `just ready` and
  `just test-lite` run no corpus suite of or-io-verify.
- The gate runs a binary's tests as threads of one process
  (`crates/tools/xtask/src/gate.rs`). Tests that share state can fail there
  and pass apart, so a binary that fails is run again, then test by test under
  nextest, whose verdict stands; when the gate names such a test, the shared
  state is a bug to fix.
- A new test that walks a corpus or an install goes in a module that is
  already slow, or `just slow-tier` is run after the next `just test`: a
  default-tier test still running at 30 s is terminated.
- `cargo test -p X --lib` runs nothing in `tests/`. Name the suite
  (`--test texture_dict`) or use `--all-targets`.
- **A clean checkout is two variables**, which `just test-no-pack` sets:
  `OPENRAGE_SCHEMA=none` and `OPENRAGE_FIXTURES` pointing at an empty
  directory. Discovery walks ancestors for a schema store and corpus, so
  unsetting a variable changes nothing, and PowerShell deletes a variable
  set to an empty string. The metadata corpus has its own variable
  (`OPENRAGE_META_CORPUS`) and is still found.
- **Fixtures are never committed.** Find them with `or_test_support::Corpus`,
  never a hand-written `read_dir`, and stage them in the sibling
  `openrage-corpus`: one file in `crates/io/or-io-verify/tests/fixtures/`
  shadows it for every format. A query without a target returns Gen8 files
  only; use `all_targets()` only where the code under test does not depend
  on the generation. A skipped suite has not passed.
- **The pre-commit hook** (`just install-hooks`, again whenever
  `crates/tools/xtask` changes) runs the chunked format check, the encoding
  lint over the index and the provenance lint, through a built `xtask` binary
  that the installer copies beside it, and compiles nothing. It refuses while
  anything is unstaged, and refuses, naming the recipe, when that copy is
  missing or older than its source (owner, 2026-10-06). Never add a check that
  compiles the workspace to it.
- ⚠ On Windows run `just fmt`, never `cargo fmt --all`: one rustfmt process
  holding every target root passes the 32,766-character command-line limit
  and fails with os error 206. `just fmt` is `cargo xtask fmt`, the same on
  every OS.

## Before merging, and how to push

Every push starts a hosted run and a `layouts` run on the owner's desktop, so
commit locally as often as you like and push once.

- **The merge rule (owner, 2026-09-25 and 2026-09-28).** When the local
  checks pass, push once, open the pull request and squash-merge it straight
  away with `gh pr merge --squash --delete-branch`. CI and `layouts` are read
  afterwards, on `main`. The local checks are one command, `just ready`, and
  a change to a format's read or write path adds that format's
  `just test-format MODULE`.
- **That is the whole list.** The pre-commit hook has already run the format
  check, the encoding lint and the provenance lint on every commit, so do not
  run them again. `just ready` already checks every package the change can
  reach, so do not run `ready-full` or `just test` after it: the slow tier is
  run on `main` between waves, and feature combinations are `main-tip.yml`'s
  `cargo hack`, after the merge.
- Rebase locally before the first push. Do not re-push because `main` moved,
  and do not close and reopen a pull request to rebase it.
- Collect review fixes into one push.
- A draft runs no hosted job, but `layouts` still runs for it on the owner's
  desktop.
- **A conflicting pull request lands by a local squash onto `main`**, with
  `(#N)` at the end of the subject, and is then closed. Never push a conflict
  resolution to a head branch. Every squash subject ends with `(#N)`: the
  janitor reads it.

## CI

Hosted CI is the cheap Linux tier by the
[owner's cost decision](docs/direction/Project_Context.md#hosted-ci-is-linux-per-pull-request-the-windows-suite-runs-on-each-tip-of-main-2026-09-23),
and the full
Windows suite runs on the self-hosted runner, which is the owner's desktop.
Nothing waits for any of it before a merge.

⚠ **`ci.yml` is on again, and `local-gate.yml` and `main-tip.yml` are still disabled** (owner, 2026-11-23,
[recorded](docs/direction/Project_Context.md#hosted-ci-is-on-again-and-the-self-hosted-workflows-stay-off-2026-11-23)):
the repository has no self-hosted runner, so `just ready` stays the gate before a merge and no run sees
the corpus. The Linux job starts on every push to a pull request and to `main`, and is read afterwards.
`gh workflow enable` restores the other two; the table says what each does when it is on.

| Workflow | When | What |
|---|---|---|
| `ci.yml`, job `linux` (hosted) | every push to a non-draft pull request and to `main` | fmt, provenance, encoding, clippy, `cargo test --workspace`, the BC7 encoder, the self-check, `or-io` for wasm32; a docs-only pull request runs the lints and the document-reading tests |
| `local-gate.yml`, job `layouts` (self-hosted) | every push to a pull request from this repository, drafts included, and to `main` | `or-io-verify` on the corpus, the behaviour baselines, the clean-checkout run |
| `main-tip.yml` (self-hosted) | every tip of `main` | compile check, tests with no pack and no corpus, clippy, BC7, GPU tests, `cargo hack --each-feature`, the shader tool |
| `ci.yml`, job `windows` (hosted) | weekly and on request | the full Windows suite on a clean machine |
| `ci.yml`, job `macos` (hosted) | weekly and on request | clippy, the workspace tests, the BC7 encoder and the self-check on a clean Apple-silicon machine |
| `release.yml` | a `v*` tag; on request, the packages with no release | `cargo xtask dist` on Windows, Linux and macOS: a `.zip`, a `.tar.gz` and a `.dmg`, each with a checksum |

- Do not move per-pull-request hosted CI back to Windows; the cost figures
  are in [Development_Workflow.md](docs/guides/Development_Workflow.md).
- CI calls cargo directly, so it needs no nextest; its one recipe step,
  `just fmt` and `just lint` on Linux, checks that the recipes start there.
  Do not add nextest to CI.
- `ci.yml` keeps `cargo fmt --all --check` on Linux on purpose, as the
  independent check that the chunked formatter covers as much as `--all`.
- ⚠ Every self-hosted job sets `CARGO_BUILD_JOBS: 4`: at the default `-j` the
  runner fails to link with `rust-lld: undefined symbol: anon.*.llvm.*`. A
  new self-hosted job takes the same cap. The same error also comes from
  stale incremental state in the runner's persistent target directories,
  which build incrementally: when no job runs, delete `debug\incremental` in
  the runner's `_cargo-target` and `_cargo-target-tip` and rerun the
  failed job (owner, 2026-10-01;
  [the runner](docs/guides/Development_Workflow.md#ci)).
- Every workflow on a `main` push cancels its predecessor. `release.yml` has
  no concurrency group and must never get one that cancels.
- Every hosted job installs toolchain nightly-2026-10-08, the pin in
  `rust-toolchain.toml`, and the self-hosted jobs build on that file;
  `crates/foundation/or-test-support/tests/toolchain_pin.rs` holds the
  workflows and this file to it.

## Evidence

- **Make a check fail once before trusting it.** A render-only egui test
  shows that drawing has no side effects and nothing about what a widget
  writes; a round trip passes when reader and writer are wrong the same way.
- Compute an expected value: call the hash function, use `offset_of!`. Never
  transcribe a hash, offset or size by hand.
- When a large number refutes a model, check the measuring code first: a
  harness bug scales with the corpus. Test rival models side by side, and
  read individual failing cases before generalising from a count.
- A derivable figure names its test; a measured one names its numerator,
  denominator, suite and revision.

## Documentation

- **Two tiers.** `docs/` documents OpenRage and is committed. `docs/extra/` is
  gitignored and holds RAGE's binary layouts and anything that relies on the
  GTA source or CodeWalker: every format specification, the
  `docs/extra/rage-base/` engine summaries and the C++ struct reference.
- Link a file under `docs/extra/` with a Markdown link and never backtick its
  path: the citation gate does not walk that tier.
- `docs/` is grouped by kind into `direction/` (records, plans and
  specifications), `reference/`, `guides/` and `features/`
  ([where a document lives](docs/guides/Documentation_Standard.md#where-a-document-lives)),
  and [docs/README.md](docs/README.md) indexes them.
- [Documentation_Standard.md](docs/guides/Documentation_Standard.md) says how a
  document is written, including figures; one document owns each topic.
- The doc gates run under `cargo nextest run --bin openrage`: `src/doc_citations.rs`
  (backticked in-tree paths exist, backticked `snake_case` names with two or
  more underscores are in code), `src/doc_counts.rs` and
  `src/doc_comments.rs`. Tests also read the CLI protocol, the plugin ABI and
  this file's toolchain line.
- Link documents with plain Markdown links, never an `@path` import, which
  loads the whole file into every session.

## Comments

A comment states what the code cannot: a constraint, an invariant, a trap, a
unit or a measurement. It never narrates the next line, argues that a change
is correct, or records where it came from (a phase, ticket, date or author):
that belongs in the commit and the pull request.

- **A public doc comment is one line.** No `# Features`, `# Usage`,
  `# Arguments`, `# Panics` or `# Returns` heading
  (`no_doc_comment_carries_a_scaffolding_heading`); `# Example` only as a
  doctest that runs. `# Safety` and `# Errors` are allowed. A module's `//!`
  header is a sentence or two.
- **Judge a comment by what it says, never by its length.** Worth keeping:
  the skinning order in `crates/runtime/or-anim/src/pose.rs`, carried counts
  in `crates/io/or-io/src/write/build.rs`, the `or-transport` note in
  `crates/runtime/or-anim/Cargo.toml`.
- ⚠ The first doc line of a `#[record(enum)]` variant that is not blank is
  description data: it is what renders a value as `Geometry (4)`
  (`every_declared_enum_variant_carries_its_note`). A record field's note
  comes from `#[note("…")]` instead.
- `src/doc_citations.rs` accepts a cited name only while code, not just a
  comment, carries it. When a comment or item goes, fix the document citing
  it; the `ELSEWHERE` list there is only for names nothing in the tree
  carries.
- `tools/restore_protective_comments.py` defines what a comment cleanup must
  put back.

## Gen8 and Gen9, Legacy and Enhanced

- Code, test names and documents say Gen8 and Gen9 for a resource's target,
  with a `G8` or `G9` suffix on a type. Anything a user reads (CLI output,
  error messages, the UI, log lines) says Legacy and Enhanced, through
  `Generation::edition_label()`.
- **A generation is not an edition:** an Enhanced install ships Gen8
  resources, so the layout comes from the file's own RSC7 version.
  `Game::GtaVLegacy` and `Game::GtaVEnhanced` name editions.
- Enhanced is a separate target. The four types Gen9 changed (the vertex
  buffer, the index buffer, the texture and the shader) are declared per
  target. A type that differs only in what it points at takes a target
  parameter, spelled `GtaDrawable<Gen9>` with no `G9` alias.
- A Gen8 struct never holds Gen9 data. A struct with per-target field
  presence was built and reverted; do not reintroduce it.
- Read support may come before write support; write support never comes
  before read.
- Before writing a parallel reader, check whether the blocker is the bytes
  or only a Rust type.

## Code conventions

- A sentence a person reads goes through `tr!`, `tr_n!` or `tr_key!`, with its
  entry in every catalog of `src/i18n/languages`; the gates run in `just ready`
  ([how a string is added](docs/reference/Crate_Reference.md#interface-languages)).
- `thiserror` for errors and `tracing` for logging, never `println!`.
  Handle errors explicitly; no `unwrap` in production code.
- RAGE types, fields and enums take the GTA source's names (D15).
- `ErrorMode` is `Lenient` or `Strict`. ⚠ A lenient dictionary read skips an
  entry it cannot read and still returns `Ok`, so a read path with no
  compiled fallback reads strict.

## The app in brief

- `AppState`'s methods are split by concern across `src/state/`, and the
  struct is deliberately flat (`the_shell_is_flat_rather_than_deep` in
  `src/doc_counts.rs`).
- A format enters the app as a `FormatHandler` in `src/state/format_handlers.rs`,
  a `PayloadHandler` in `src/state/payload_handlers.rs` and a
  `FormatCapabilities` row in `src/state/capabilities.rs`; a named kind adds a
  `DocumentKind` variant and a `KindInfo` row in `src/state/kinds.rs`
  ([what a format costs the app](docs/reference/Crate_Reference.md#what-a-format-costs-the-app)).
- Panels are a registry keyed by a string id (`src/ui/panel_registry.rs`,
  registered in `src/ui/panels/registry.rs`), and `src/ui/dock_layout.rs`
  persists a dock arrangement.
- `src/ui/timeline/` is a format-agnostic widget (`TimelineSource`) shared by
  the particle and animation timelines.

The module table and the app's traps are in
[Crate_Reference.md](docs/reference/Crate_Reference.md).

## Format traps

- ⚠ A Gen9 resource read with the Gen8 layout does not fail: it returns
  plausible nonsense, such as 0×0 textures with no format. The file's RSC7
  version picks the layout.
- A field named as padding can hold live data. Measure what the bytes hold
  before zeroing them, and preserve what you cannot name.
- `.ymt` is PSO, RBM or RBF0: dispatch on the leading magic, not the
  extension.
- Skinning is `world * inverse_bind`, in that order; the reverse explodes the
  mesh.
- A declared container type is a claim about the base class: measure the
  entry stride before choosing the element type.
- The ranked list of the rest is in
  [New_Format_Implementation_Guide.md](docs/guides/New_Format_Implementation_Guide.md).

## Other documents

- [docs/README.md](docs/README.md): every document in `docs/` under its folder.
- [Private_Tier.md](docs/guides/Private_Tier.md): what a clone lacks, why, and how
  to reconstitute it.
- [Licence_And_Provenance_Policy.md](docs/guides/Licence_And_Provenance_Policy.md)
  and [LICENSE-third-party.md](LICENSE-third-party.md): what the tree derives
  from, what that permits, and third-party notices.
- [Oracle_Testimony.md](docs/guides/Oracle_Testimony.md): the recorded testimony and
  the check against it.
- [Interface_Languages.md](docs/guides/Interface_Languages.md): making or changing a
  language of the screens, and the words the French catalog uses.
- [BCn_Encoding_And_Mipmaps.md](docs/features/BCn_Encoding_And_Mipmaps.md): texture
  authoring.
- [Shader_Definition_Database.md](docs/features/Shader_Definition_Database.md): what a
  material's parameters are.
- [Animation_Preview.md](docs/features/Animation_Preview.md) and
  [YCD_Channel_Encoding.md](docs/features/YCD_Channel_Encoding.md): clip playback and
  writing channels back.
- [Particle_Preview.md](docs/features/Particle_Preview.md) and
  [Particle_Shader_ABI.md](docs/reference/Particle_Shader_ABI.md): the particle preview
  and its shader contract.
- [File_Support.md](docs/features/File_Support.md),
  [Viewport_Shading_Modes.md](docs/features/Viewport_Shading_Modes.md),
  [Mesh_Interchange.md](docs/features/Mesh_Interchange.md) and
  [Dlc_Pack_Creation.md](docs/features/Dlc_Pack_Creation.md): what each kind of file
  supports per game, shading modes, glTF out and back, and generating a DLC pack.
- [GTA_VI_World_Viewer.md](docs/features/GTA_VI_World_Viewer.md): the World screen, how a
  GTA VI install is mounted with no table to read, and the streaming rules.
- [CLI_Server_Protocol.md](docs/reference/CLI_Server_Protocol.md), with
  [openrage_client.go](docs/reference/openrage_client.go), and
  [Plugin_ABI.md](docs/reference/Plugin_ABI.md): the `openrage-cli serve` protocol and
  the plugin boundary, both read by tests.
- [MCP_Server.md](docs/reference/MCP_Server.md): `openrage-cli mcp`, the server an AI
  agent drives OpenRage through, how a client is given it, and its tools.
- [README.md](README.md), [plugins/README.md](plugins/README.md),
  [tools/cw-compare/README.md](tools/cw-compare/README.md),
  [assets/logos/README.md](assets/logos/README.md), and the corpus and
  snapshot staging notes in
  [crates/io/or-io-verify/tests/fixtures/README.md](crates/io/or-io-verify/tests/fixtures/README.md)
  and [crates/io/or-io/src/meta/README.md](crates/io/or-io/src/meta/README.md).
- The local, gitignored [docs/extra/README.md](docs/extra/README.md) indexes
  the RAGE layout specifications.
