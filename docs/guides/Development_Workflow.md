# Development workflow

How work is done on this repository and on the owner's machine: lanes, build
output and cleanup, the limits on agent work, the local checks, CI, and how a
change lands.

[AGENTS.md](../../AGENTS.md) states the rules every session obeys; this document
is their detail. It describes the [justfile](../../justfile), `scripts/` and
`.github/workflows/`, which win wherever the two disagree. Dated decisions are
in [Project_Context.md](../direction/Project_Context.md#decisions).

## The machine

One Windows desktop runs the agent lanes, the self-hosted CI runner and the
owner's own work, games included.

| Path | What it is |
|---|---|
| `openrage` | The checkout, the owner's working copy. Read `git status` before touching it; never switch its branch, clean it or discard what it holds |
| `openrage-lanes`, beside the checkout | The lane root: every agent worktree, and nothing else |
| `openrage-corpus`, beside the checkout | Game files for the corpus suites, the testimony and the metadata schema store: an unversioned directory |
| `openrage-runner`, outside that folder | The self-hosted runner, its target directories and its tools |

The folder the checkout is in is the organisation's folder and holds unrelated
projects. Nothing a session runs may open a window (AGENTS.md).

## Lanes

A lane is a git worktree under the lane root, on its own branch, with its own
`target/`. Every agent works in one, and a lane holds one agent. Make one from
the checkout:

```powershell
just agent-worktree r66d-ymap
```

It adds a worktree at `..\openrage-lanes\NAME` on branch `agent/NAME`, from
the checkout's current HEAD, and writes a `.agent-worktree` marker. If that
HEAD is not where the lane should start, run `git fetch` and
`git reset --hard origin/main` in the new lane while it is still empty. From
inside a lane the recipe resolves the lane root one level too deep, so run it
from the checkout only.

The justfile keys one variable off the marker:

| Variable | In a lane | Elsewhere |
|---|---|---|
| `RUSTC_WRAPPER` | sccache, where it is installed | empty |

A lane is incremental like the checkout, because `.cargo/config.toml` says
`incremental = true` and sccache passes an incremental crate straight through:
the workspace crates compile incrementally and the registry crates come from
sccache. The justfile exports neither `CARGO_TARGET_DIR` nor
`CARGO_INCREMENTAL`: sccache hashes every `CARGO_*` variable into its key, so a
per-lane target directory in the environment made every lane miss every other
lane's entries, and it aborts on `CARGO_INCREMENTAL=1`. Measured 2026-10-08
with a private server and `png` built into two target directories: 0 of 10
hits with `CARGO_TARGET_DIR` in the environment, 10 of 10 with `--target-dir`.
A lane's target directory is cargo's default, `target/` under the worktree, and
the runner's ambient `CARGO_TARGET_DIR` passes through untouched.

- ⚠ Leave `CARGO_TARGET_DIR` and `CARGO_INCREMENTAL` unset in a lane. A target directory shared
  between worktrees serves one lane another's stale build of a crate such as
  `or-io-macros`, and its suites go green measuring nothing.
- Do not set `RUSTFLAGS` without saying so: it changes every sccache key, and
  a per-worktree `--remap-path-prefix` makes every lane miss the cache.
- A lane's workspace crates never hit another lane's sccache entries: their
  absolute paths reach the compiled output and so every downstream key.
  Measured 2026-09-28 with sccache 0.17, a private server and two copies of
  one tree at different paths: `or-io` missed with `SCCACHE_BASEDIRS` in
  server and in client-side mode, and adding a per-copy
  `--remap-path-prefix` gave 0 hits in 162 compiles, third-party crates
  included, because basedirs does not strip a path inside that flag.
- Besides the lane itself, an agent writes only what a recipe writes on
  purpose (`just snapshot` into the corpus's schema store and
  `just baselines-record` into the corpus's baselines) and scratch under
  `$env:TEMP`.
- A new lane holds tracked content only: `docs/extra/` and anything else
  gitignored is absent until copied in.

## Cleaning up: the janitor

`just janitor` removes finished lanes and their build output and reports what
it freed; `just janitor-dry` prints the same report and removes nothing. Both
run from the checkout only, since a process cannot delete the directory it
runs in. A worktree is removed only when all of these hold, and every other
one is listed with the reason it was held:

- it is not the checkout, is not locked and is not on a detached HEAD;
- it has nothing uncommitted or untracked;
- it holds no ignored file that no branch carries and the checkout lacks,
  which is what protects a specification written in the gitignored layout
  tier;
- nothing has written to it for an hour;
- its work has landed.

Landed means one of three things, asked in this order: its HEAD is an
ancestor of `origin/main`; a pull request recorded this exact HEAD and was
merged; or such a pull request was closed and `main` carries its `(#N)`
subject, which is how a conflicting pull request lands by local squash. Every
pull request here is squash-merged, so the ancestor test alone never
recognises a finished lane, and an open pull request at this HEAD holds it.
The GitHub questions need an authenticated `gh`; without one, or with
`-NoGitHub`, every squash-merged lane is held and the report says the question
went unasked. A lane left dirty is held for good, since the janitor cannot tell
an abandoned lane from one mid-thought: read what the report holds, not only
what it freed.

The janitor also names every worktree outside the lane root, deletes residue
under the lane root that git no longer knows about, and reports but never
deletes a plain file sitting directly in the lane root or an orphaned cargo
target directory in the organisation's folder.
`just xtask-selftest` falsifies the landing detection offline.
`just disk-budget` fails when the lane root passes 45 GiB. It counts open
lanes, not free space, and is not part of `just ready`: a clone with no lane
root would pass it by having nothing.

The janitor is `cargo xtask janitor`, written once in Rust and run the same on
Windows, Linux and macOS, so it no longer depends on which PowerShell started
`just`. It finds the lane root from the checkout, so it also runs from a lane,
which holds itself. It no longer prints the free space on the drive, which
needs a platform call nothing else here makes.

## Deleting a branch once it has landed

Landed branches are deleted, locally and on `origin` (owner decision,
2026-09-25). A landed branch's work is on `main` under its `(#N)` subject, so
removing a landed lane stays safe once its branch is gone. The janitor itself
never deletes a branch.

1. Merge with `gh pr merge <N> --squash --delete-branch`, which deletes the
   remote branch.
2. Once the janitor has removed the lane, delete the local branch with
   `git branch -D agent/NAME`. Git refuses to delete a branch a worktree still
   has checked out, and a squash-merged branch is never an ancestor of
   `main`, so `-d` refuses it once its upstream is gone.
3. If the remote branch survived (a landing by local squash, a merge
   without `--delete-branch`, or a `gh` run that stopped at a local branch a
   lane still has checked out), delete it with
   `git push origin --delete agent/NAME`; `git fetch --prune` then drops the
   stale tracking refs.

⚠ `-D` deletes whatever it is given, so delete only a branch the janitor's
report or its pull request shows as landed; the report names each removed
lane's branch and the pull request it landed by. A branch whose work is not
on `main` stays unless the owner says otherwise: `feat/ydd-clothing-preview`,
the source of [R46](../direction/Roadmap.md#r46-first-plugin-a-clothing-tool-on-openrage), is one, and the tag
`archive/ydd-clothing-preview` keeps its work reachable.

## What agent work may take from the desktop

Agent work runs at full speed by default. `just limits-start` turns the
limits on, so the owner can go on using the machine, until `just limits-stop`
or a restart; nothing starts them at logon (owner decision, 2026-09-28). It
starts `OpenRage agent limits`, an on-demand per-user task that runs
`scripts/agent_limits.ps1` with `-Run`, and the watcher puts agent processes
in Job Objects:

| | |
|---|---|
| CPU | Idle priority, inherited by every child at creation |
| I/O and page priority | very low, also inherited |
| Memory, per process | a quarter of installed RAM, enforced by the kernel |
| Memory, all agent work | half of installed RAM, summed by the watcher every second |
| CPU rate | uncapped unless `just limits-cap N` sets a percentage |

A process is agent work when it is already in a watcher job, when its parent
is, when its image or working directory is under the lane root or an
`-ExtraRoots` entry, or when its environment carries `CURSOR_AGENT=1` or
`CLAUDECODE=1`, which every agent shell passes down. The owner's own terminal
carries no marker, but anything he starts inside a lane directory counts.

A process past its ceiling has the allocation refused and, if it survives
that, is killed with exit code 137; at the all-agent ceiling the watcher kills
the largest agent process. Every kill is a line in
`target/agent-limits/limits.log` in the checkout, so an exit code of 137 in a
lane is the cap, not a crash.

| Recipe | Does |
|---|---|
| `just limits-install` | registers the task with no trigger and starts nothing; from the checkout only, since the task runs the script where it was installed. Add `-ExtraRoots` with the runner's directory to cap the self-hosted runner too, as this machine's task does |
| `just limits-start` | starts the watcher through its task, until `limits-stop` or a restart |
| `just limits-stop` | stops it, lifts every ceiling and restores enrolled processes to Normal, until the next start |
| `just limits-status` | what is enrolled, and what was killed |
| `just limits-dry` | what it would enrol now, and why, touching nothing |
| `just limits-cap N` | hard-caps each agent command's process tree at N% of the machine while the limits are on; `0` removes it; the setting persists across restarts |
| `just limits-selftest` | falsifies the watcher against hog processes it launches itself; about 20 s, safe beside the live watcher |
| `just limits-uninstall` | stops the watcher and removes the task |

A running watcher keeps the code it started with, so after pulling a change to
the script run `just limits-stop` and then `just limits-start`. Cursor runs
each agent command in a job of its own, so the watcher nests one job per
command tree: a CPU cap applies per tree, not in total, and the all-agent
ceiling is polled rather than enforced by the kernel. There is no default CPU
cap, because Idle priority already yields every cycle a game asks for;
`just limits-cap 50` suits a session where shared cache, memory bandwidth and
heat matter more than build time. An agent with no marker working outside the
lane root, or anything elevated, escapes the watcher.

## Committing: the pre-commit hook

`just install-hooks` copies `.githooks/pre-commit`, a short shell script, and
the `xtask` binary it runs into the repository's hooks directory, which every
worktree shares, and a commit then runs three checks that compile nothing:

- `cargo xtask fmt --check`, the chunked formatter;
- `cargo xtask lint-encoding --staged`, which reads the index and so judges the
  commit rather than the disk (`just xtask-selftest` falsifies that scope);
- `cargo xtask lint-provenance`: committed game content and source-tree
  paths and citations are fatal in every tracked file.

They are Rust, in `crates/tools/xtask`, and run the same on Windows, Linux and
macOS with no PowerShell. The hook refuses while anything is unstaged and warns
about untracked `.rs` files, because the formatter reads the working tree rather
than the commit. An installed binary is a copy, so a change to `xtask` reaches a
machine only when `just install-hooks` runs there again; the installer records
a hash of the source, and the hook refuses, naming the recipe, when the hash
differs from the index or the binary is missing, where it once skipped two of
the checks in silence. A check that compiles the workspace would make the hook
take minutes, a slow hook gets skipped with `--no-verify`, and a skipped hook
lets UTF-8 BOMs reach `main`: clippy and the tests stay in `just ready`, which
therefore does not run these three again.

## Formatting is chunked

`just fmt` checks and `just fmt-fix` rewrites every workspace target through
`cargo xtask fmt`. `cargo fmt --all` hands one rustfmt process
every target root on one command line, so every new target and every
character of a lane's path spends margin against the 32,766-character Windows
limit, and a lane with a long name hits it first. The task runs the same
rustfmt over the same roots in as many processes as a per-invocation budget
allows, so a new target never moves the ceiling.

- `--list-targets` prints the set it formats, and `just fmt-selftest` asserts
  it reaches the same files `cargo fmt --all` does; on Windows its test also
  confirms a synthetic workspace really breaks `cargo fmt --all`.
- Only the check runs chunks concurrently: two roots can reach one module,
  and two processes rewriting one file race, so `fmt-fix` is single-process.

## Recipes and test tiers

`just --list` shows every recipe. These are the ones a change is checked with:

| Recipe | Runs |
|---|---|
| `just ready` | the merge rule's checks over what the change touches ([the gate](#the-gate-and-the-two-tiers)): clippy, and the default test tier with each test binary run once |
| `just ready-full` | the same over every package, whatever changed |
| `just test-affected [PART…]` | `ready` without clippy, kept to the tests whose names hold a PART |
| `just test-format MODULE` | one format's or-io-verify corpus suite and its owned-cost test, slow tier and ignored tests included |
| `just slow-tier` | rewrite the slow tier in `.config/nextest.toml` from the timings of the last `just test` |
| `just self-check` | the app's startup and every panel through real egui frames with no window; `--viewport` adds a surfaceless GPU device, `--open PATH` with it opens a file through the real load path and draws it in its workspace, and `--frames N` sets the frames per unit |
| `just shots SCRIPT DIR` | the same frames driven by a script of steps, each `shot` saved as a PNG in `DIR` ([screenshots](#screenshots)) |
| `just check` | `cargo check --workspace` |
| `just lint` | clippy with `--all-targets` and `-D warnings`; `--all-targets` is what reaches tests and `cfg(test)` modules |
| `just test-crate X` | one crate's lib tests of the default tier under nextest, and nothing in that crate's `tests/` |
| `just test-lite` | lib and bin tests and doctests; no integration test binary, corpus-backed or not |
| `just test` | every test of the workspace under nextest (`--profile all`), doctests, then the skip ledger |
| `just test-cargo` | the same tests through `cargo test`, for a machine without nextest; about twice the wall time |
| `just test-strict` | `cargo test --workspace` with `OPENRAGE_REQUIRE_FIXTURES=1`, so a corpus skip fails |
| `just test-no-pack` | the workspace as a clean checkout sees it |
| `just skips` | the last run's skip report again, running nothing |

### The gate and the two tiers

The owner's rule of 2026-11-20 is that an agent tests the least that covers
its change and that a default run takes about 30 s
([recorded](../direction/Project_Context.md#an-agent-tests-what-its-change-touches-and-a-default-run-takes-thirty-seconds-2026-11-20)).
`just ready` is `cargo xtask gate` (`crates/tools/xtask/src/gate.rs`), and it
is scoped three ways:

- **To what the change touches.** The files that differ from the merge base
  with `origin/main`, untracked ones included, are mapped to their packages,
  and every package that depends on one is added
  (`crates/tools/xtask/src/affected.rs`). A file in no crate (a document, the
  justfile, a workflow) belongs to the app, which holds the document gates,
  and to each crate whose sources name it. The root manifest, the lock file,
  the toolchain file and `.cargo/` reach every package. `--since REF` on the
  task compares against another ref, so `cargo xtask gate --since HEAD` checks
  only what is not yet committed.
- **To the default tier.** `.config/nextest.toml` names the slow tier in a
  marked block, by module: the modules that cost the most per test are left
  out until the rest fit a budget, and the gates over the documents, the
  sentences and the source, with the app's self-check test, stay whatever
  they cost (`crates/tools/xtask/src/slow_tier.rs`). `just slow-tier` rewrites
  the block from `target/nextest/all/junit.xml`, which `just test` leaves. A
  bare `cargo nextest run` takes the default tier too and ends a test still
  running at 30 s; `--profile all` takes every test.
- **To one process per test binary.** The gate runs each binary once with its
  tests as threads, where nextest starts a process per test. A binary that
  fails is run a second time, and then test by test under nextest, whose
  verdict stands: tests that share state fail together and pass apart, and on
  Windows a panic ends the whole process, because Cranelift cannot unwind.

Clippy runs beside the build and the tests in a build directory of its own,
`target/lint`, about 4 GiB, because cargo locks a build directory.

Measured on 2026-11-20 at `4a53f136`, 16 cores and 32 threads, with a staged
corpus and warm sccache:

| What | Before | Now |
|---|---|---|
| `just ready-full` in a new lane, nothing built | 1,180 s: clippy 42 s, test build 146 s, 7,727 tests 925 s, doctests 16 s, self-check 51 s | the same build, then 5,358 tests in 12 s |
| `just ready` in a new lane after one line of the app changed, nothing built | not measured | 144 s, 129 s of it the build, leaving 6 GiB of `target/` where the whole workspace leaves 59 GiB |
| `just ready-full`, built lane, nothing changed | the 925 s of tests again | 15 to 22 s |
| `just ready` after a document changed | clippy, and the app's 2,647 tests in 89 s | 12 to 13 s |
| `just ready` after one line of the app changed | clippy 39 s, then the test build 20 s, then the tests | 30 s |
| `just ready` after one line of `or-anim` changed | as above over the workspace | 45 to 60 s |
| `just ready` after one line of `or-io` changed | as above | 258 s, 240 s of it building 23 packages and their test binaries |

At `4a53f136` the app's default tier took 3.5 s as one process and 14 s under
`cargo nextest run -p openrage`, a process per test. What is left after an
edit is the compiler: the floor is the rebuild
of the edited crate and of what depends on it, which for `or-io` is nearly
every test binary of the workspace.

`just test` runs every test, about twenty minutes and 27,915 s of summed test
time, so it is run on `main` between waves, by whoever coordinates them, and
is never a lane's gate; a failure there is traced to the merge that caused it.

`just test` finishes when its longest test does, so `.config/nextest.toml` starts
the corpus packages before the unit tests and the longest named tests before
those, and a corpus loop that is one test shares its files between the cores
instead of walking them in turn (`rayon` in `or-io-verify`'s gates and `meta`
suites, one thread per file where a test holds a guest, and eight processes for
`the_target_sample`, whose sample works in the current directory). A new test
that walks the whole corpus in one loop does the same.

A test build resolves the configuration to a scratch folder, `openrage-test-` and
a random suffix under `$env:TEMP`, that the process removes as it exits, a
failed test's included, so a nextest run leaves none per test
(`a_test_process_leaves_no_folder_in_the_temporary_folder_when_it_ends`). Each
test thread's `prefs.json` is a folder of its own under it, so a test that saves
a preference leaves the next `AppState::default()` as it was
(`a_test_that_saves_a_preference_leaves_the_next_default_state_as_it_was`). A test
that needs a folder of its own takes `tempfile::tempdir()`, never a fixed name
under the temporary folder, and
`no_test_makes_a_folder_in_the_temporary_folder_that_nothing_removes` refuses a
call that does, in any crate.

The rest: `lint-provenance`, `lint-provenance-history` (R66g's scan of every
ref of a mirror), `lint-features`, `snapshot` and
`snapshot-status`, `cw-roundtrip` (a CodeWalker
comparison, a diagnostic rather than a gate) and the shader-tool recipes. The
documentation gates run under `cargo nextest run --bin openrage`;
[Documentation_Standard.md](Documentation_Standard.md) lists them.

A recipe is a call to `cargo` or to `cargo xtask`, so on Linux and macOS put
`just` on `PATH` beside cargo-nextest; no PowerShell is needed. Windows runs
them under `powershell.exe`, as it always has, and hosted CI's Linux job runs
`just fmt` and `just lint` to show they start there. The janitor, the disk
budget, the agent limits and `cw-roundtrip` still run a PowerShell script, so
they exist on Windows only (R58a ports the janitor next). Started on Linux without a library winit or wgpu
loads at run time, the app exits naming each one and its Debian and Fedora
package (`src/platform/runtime_libraries.rs`). Building there needs ALSA's
headers, `libasound2-dev` on Debian and `alsa-lib-devel` on Fedora, because
`cpal` links ALSA for sound; hosted CI's Linux job installs them.

`just test-cargo` is the only fallback without nextest, because there the two
selections are the same tests; `test-lite` has none. nextest runs no doctests,
hence the recipes' `cargo test --workspace --doc`, and CI never uses it.

### The skip ledger

A skipped suite has not passed, and libtest discards a passing test's output,
so a printed skip notice is never seen. Every skip goes through the
`or_test_support` skip helpers into a per-run ledger instead, and
`skip-ledger` totals it by reason at the end of `test`, `test-cargo`,
`test-strict`, `test-no-pack` and `test-lite`, and of both hosted CI jobs. Do
not print ad-hoc skip lines.

- `test-lite` excludes whole binaries, so nothing inside them can record a
  skip; it lists what its filter left out with `nextest list` and folds that
  into the ledger as `tier`.
- Every recipe drains the ledger it filled, or the next run would report the
  leftovers as its own (`every_workspace_run_owns_its_skips`).
- The ledger finds the target directory by walking for `.fingerprint`, which
  cargo maintains, rather than relying on `CACHEDIR.TAG`, which cargo writes
  only when it creates the directory
  (`a_running_test_binary_can_find_the_ledger`). A ledger it cannot find is
  reported as unread, never as empty.

### The clean-checkout state

`just test-no-pack` sets the two variables AGENTS.md names for a clean
checkout, and `the_recipe_and_the_gate_ask_the_same_question` holds it to the
`layouts` job's no-pack step. The metadata suites find their own corpus
through `OPENRAGE_META_CORPUS`, not `OPENRAGE_FIXTURES`, so they still run
there. In a lane, do not set `OPENRAGE_FIXTURES` at all: the sibling
`openrage-corpus` is discovered. An `OPENRAGE_FIXTURES` naming a directory
that does not exist panics rather than skipping, because it would be green
over nothing; an absent corpus skips.

⚠ The no-pack step of `.github/workflows/main-tip.yml` sets no
`OPENRAGE_SCHEMA`, so it does not model a clean checkout yet: it still finds
the schema snapshot.

### Cross-checking another OS

`just cross-check windows` or `just cross-check linux` lints the whole workspace, tests included, for `x86_64-pc-windows-msvc` or `x86_64-unknown-linux-gnu` from a Mac or a Linux machine (`cargo xtask cross-check`, which takes `--check` for `cargo check` and `-- -p CRATE --features F` to name what to lint). The Rust is compiled for the target as it would be there; the C and C++ that dependencies bring (`ring`, the shader translator) is compiled by Zig, which it finds as `zig` on the path, in `ZIG`, in `--zig`, or in the `ziglang` Python package (`pip install ziglang` into a virtual environment, then `--zig "/path/to/venv/bin/python -m ziglang"`). It compiles and does not link, so it says the code builds there and not that it runs there, and Zig's clang and mingw headers are not MSVC's `cl` or a distribution's gcc. At `786e9ce4` both targets pass clippy with `-D warnings`.

### Running on Linux from a Mac

A Lima virtual machine runs the workspace on real Linux, which a cross-check cannot: `limactl create --name=linux --vm-type=vz template:ubuntu-lts`, a read-only mount of the game install and a writable share in `lima.yaml`, Mesa's Vulkan drivers (`mesa-vulkan-drivers`, whose lavapipe is a software Vulkan device) and `rustup` inside it. Copy the tree in with `git archive HEAD`, build there, and `WGPU_BACKEND=vulkan` makes `or-shader-tool render-census` use the Vulkan route; `--self-check --viewport` and `--shots` run headless on lavapipe. On a Mac the same route runs through MoltenVK: build `or-shader-tool` with `--features render-drawable,vulkan-portability`, set `DYLD_FALLBACK_LIBRARY_PATH` to the Vulkan loader's folder and `VK_ICD_FILENAMES` to MoltenVK's `icd.d` file, and `WGPU_VALIDATION=1` with `VK_LAYER_PATH` adds the Khronos validation layer.

### Screenshots

Agents never open a window, so what a change draws is seen through
`just shots SCRIPT DIR`. It runs the editor's own frame (the real `AppState`,
`ui::draw`, the overlays and the viewport on a headless wgpu device) from a
script, and saves each `shot` as a PNG in `DIR`. Look at before and after
screens of a change to what the app draws, and say in the pull request what
they show: `gh` cannot attach an image, so the PNGs stay under `$env:TEMP` for
whoever reviews the change. It needs a GPU adapter, so CI does not run it.

The configuration is resolved as the self-check's is: a scratch directory with
no preferences, which is a first run, unless `OPENRAGE_CONFIG_DIR` names one.
Keys are then extracted from the install's executable, so give the `game` step a
`wait` before opening archives. Set `OPENRAGE_SCHEMA=none` to draw metadata as a
clean checkout sees it.

The script is one step per line, and `#` starts a comment. It is read whole
before anything is drawn, so a wrong line fails at once with its number.
Coordinates are points on a 1600×900 screen unless `size` changes it.

| Step | Does |
|---|---|
| `size W H` | sets the screen, in points |
| `game legacy` or `game enhanced`, then an optional folder | makes the install active, as the start page does, and waits for its rows, so the next step finds them; with no folder, the one discovery finds |
| `game gta6 FOLDER` | sets the GTA VI folder, as Settings ▸ Games does, and mounts nothing: a GTA VI install is read by scan, and the World screen then indexes it |
| `wait SECONDS` | draws frames while background work (keys, the name scan, loads) runs |
| `open PATH` | loads a file through the real load path and waits for it, or opens a folder as Open Folder does and waits for its rows, and for the banner that offers to make it a project to say what the folder holds, so the next step finds them; a file of the other edition than the active game raises the question that ends in "Yes, switch to GTA V (Enhanced)" (`src/app/modals.rs`), so put `game legacy` or `game enhanced` before the `open` or click that button after it |
| `project PATH` | opens a `.openrage-proj` as Open Project does once its file is picked, and waits for its files and source folders to be read |
| `extract FOLDER` | extracts the rows selected in the active install, folder or archive into `FOLDER`, as Extract… does once its folder is picked, and does not wait for it; give it a folder under `$env:TEMP` |
| `gltf PATH` | reads the `.glb` or `.gltf` at `PATH` over the active model, as Replace from glTF… does once its file is picked, and leaves its question open |
| `pick PATH` | has the next file dialog pick `PATH`; a dialog no `pick` answers is cancelled |
| `workspace NAME`, `reset` | switches workspace; restores its default layout |
| `mode NAME` | picks the viewport's shading mode as its pill does: `wireframe`, `solid`, `material`, `shaded` or `game`, which draws with the SPIR-V pack that `OPENRAGE_GAME_PACK` names (the mode is not offered without one) |
| `click X Y [ctrl] [shift] [alt]`, `rclick X Y [ctrl] [shift] [alt]`, `dclick X Y [ctrl] [shift] [alt]`, `hover X Y`, `scroll X Y DY` | pointer input; a click holds the modifiers it names, so `click 800 450 alt` is the Alt click that selects a model in the viewport |
| `type TEXT`, `key NAME [ctrl] [shift] [alt]` | keyboard input, with egui's key names (`Escape`, `Enter`, `F2`) |
| `palette`, `closeall`, `frames N` | opens the command palette; closes every document, asking first when one is unsaved; draws N frames |
| `startpage`, `closeproject`, `newproject TEMPLATE [details [FOLDER]]` | raises the start page over the shell, as View ▸ Start page does; closes the open project, as Close project does, which asks first when the project has a change nothing wrote down, and raises the start page; opens the New project dialog on `empty`, `files`, `clothing`, `weapon`, `particle`, `map` or `adopt`, on its gallery or, with `details`, on the name and folder a goal card opens it on, and with a `FOLDER` to adopt, as its picker would choose it |
| `drop PATH` | drops the file or folder at `PATH` from outside the window, as Explorer does, on the middle of the Wardrobe's panel, or of the weapons list on the Weapons screen, or of the Effects panel on the Particle screen, or of the map list on the Map screen, when that is on screen and else on a point of the window; the drop is Add clothes where the Wardrobe has it, Add weapons where the Weapons screen has it, Add particle effects where the Particle screen has it, Add map files where the Map screen has it, an import on the Project workspace (a folder is walked) and an open on every other screen |
| `droptile N PATH` | drops the file at `PATH` from outside the window on tile `N` (counting from 0) of the texture variations under the dressed ped, as Explorer does, which makes that texture from the image |
| `hoverfile PATH`, `hoverend` | holds the file or folder at `PATH` over the window, as Explorer does while a drag has not let go, so the frames report a hovered file and the drop hint draws; it stays until `hoverend` or a `drop` |
| `askproject PATH` | asks for the project at `PATH` as a recent project's card does, without waiting: with a project open it raises the question whether it opens here or in a new window, and without one it opens |
| `projectmenu` | opens the menu the project's name in the status bar opens, with the other recent projects |
| `build`, `hold`, `release` | starts every build of the open project, as Build all does, without waiting for it; `hold` stops the pollers that take a worker's result, so a build or a cook that has finished still reads as running and the Build tab, the status bar and the job list draw it; `release` starts them again |
| `theme dark`, `theme light`, `scale PERCENT` | changes the appearance |
| `language CODE` or `language system` | changes the interface language as the Settings page does, from the next frame: `en`, `fr` or the code of a file in the `languages` folder of the configuration; a `settings` page is still named by its English label, since the script is read before any language step runs |
| `shot NAME` | saves `NAME.png` |
| `picker NAME` | saves `NAME.png` of Browse with nothing open, which is the start page |
| `settings NAME PAGE` | draws the Settings page with that label, at its own window size |

App shortcuts such as Ctrl+S are dispatched from winit, so no step reaches
them.

Nothing a script clicks reaches past the app. A self-check's first act,
`stand_in_for_the_desktop`, sends every call that would to its stand-in in
`src/platform/`, which logs what it was asked: the run prints it and the Log
panel shows it. Reveal in Windows Explorer and Open folder open no window, a
copy leaves the clipboard as it was, a file dialog picks what the last `pick`
named or is cancelled, and the File associations page reads and writes no
registry. A stream plays to a silent sink that holds only what is sounding,
and Bake game shaders, and the bake a first launch starts, ask a stand-in that
bakes nothing; the game's shader pack is the scratch
configuration's. Rows a copy writes out stage in the run's own
folder, and the toast of a delete or a copy, and the log of a bake, says what its stand-in did.
⚠ A delete leaves the file where it is, so the row it took comes back when its
folder is read again.

The Project and Wardrobe screens are also held by a record, which a test can compare and a script cannot:
`the_task_screens_draw_as_they_did_before_the_table` draws them, the start page and the
New project dialog at 1280 by 720 and compares every named widget and its rectangle with
`src/ui/screens_before_the_table_of_task_screens.txt`. A change that means to draw them
differently records it again with `OPENRAGE_RECORD_SCREENS=1 just --command cargo nextest run --bin openrage the_task_screens_draw_as_they_did`,
and the pull request says what the new record shows. The record holds no text of the machine: a
message that names a path says `<path>`, the comparison ignores CRLF line ends, and the rows
a long scratch folder adds to a message are taken out of what lays out after it, so the record
holds in any scratch folder that breaks a message into two lines at most, and a longer one stops
the test and says so.

## The nightly Cranelift build

`rust-toolchain.toml` pins a dated nightly, and `.cargo/config.toml` builds the
dev and test profiles with Cranelift code generation and `-Zthreads=8`.
Release and every other profile stay on LLVM. rustup installs the pinned
toolchain, with its Cranelift component and the wasm32 target, on the first
cargo command in a checkout.

Measured 2026-10-08 on 16 cores and 32 threads with nothing else building, a
cold `cargo test --bin openrage --no-run` without incremental: stable 1.95.0
146 s, nightly 129 s, nightly with `-Zthreads=8` 117 s, Cranelift 66 s,
Cranelift with `-Zthreads=8` 45 s. An edit rebuild is no faster (8 to 16 s for
`src/`, 27 to 34 s for `or-io`): that time is the incremental analysis and the
link, which neither option touches.

- Cranelift cannot unwind a panic on Windows, so a test that panics on purpose
  kills a process shared with other tests: run the app's tests with `cargo nextest run`,
  one process per test, never `cargo test --bin openrage`. Six of the app's tests
  were removed for that reason or because they decoded mp3 with SIMD Cranelift
  cannot compile: those of the panic hook, the log lock, the temporary-folder
  cleanup and the mp3 replacement.
- Cranelift emits unoptimised code and ignores `opt-level`, so a hot loop runs
  slower; the crates the dev profile built at `opt-level = 3` stay on LLVM.
  So does the wasm interpreter, `wasmi`, at `opt-level = 1`: the slowest plugin
  test is 190 s on Cranelift and 9 s on LLVM, and a lane takes a registry crate
  from sccache.
- The workspace crates the corpus suites spend their time in (`or-io`,
  `or-io-verify`, `or-anim`, `or-source`, `or-test-support`, `or-plugin`,
  `or-wardrobe`, `or-world`) were measured on LLVM and left on Cranelift.
  Measured 2026-11-20 on the same machine with the corpus staged and the lane
  built, every test of the workspace, the slow tier included, 7,640 of them,
  before the tiers were split: 440 s on Cranelift
  and 239 s with those crates and `wasmi` on LLVM at `opt-level = 1`, 13,355 s
  and 6,893 s of summed test time. The cost is the cold build every lane pays:
  `cargo test --bin openrage --no-run` after cleaning those crates is 57 s on
  Cranelift and 154 s with them on LLVM, while an `or-io` edit rebuild is 36 to
  46 s before and 37 to 51 s after. Worth it where the target directory is
  kept, as on the runner, and not for a fresh lane, whose gate runs no corpus
  suite. `opt-level` 2 or 3 on one of them does not link (`undefined symbol:
  __declspec(dllimport) generic_array::hex::LOWER_CHARS`), and `egui` with its
  font crates on LLVM ends the app's tests in an access violation.
- Cranelift aborts the process on an x86 SIMD intrinsic it does not implement.
  The config keeps the crates that use or call one on LLVM; a new dependency
  with its own SIMD can add to that list, and the symptom is `is not yet
  supported` followed by `non-unwinding panic`.
- `ui::every_menu_item_is_in_the_palette` fails on this nightly with LLVM or
  Cranelift (the walk does not reach the Project menu) and passed on 1.95.0.
- Moving the pin means the date in `rust-toolchain.toml`, in every workflow and
  in AGENTS.md; `toolchain_pin.rs` holds them together.

## CI

⚠ `ci.yml` is on again (owner, 2026-11-23; see
[Project_Context.md](../direction/Project_Context.md#hosted-ci-is-on-again-and-the-self-hosted-workflows-stay-off-2026-11-23)).
`local-gate.yml` and `main-tip.yml` stay disabled until the repository has a self-hosted runner and
`gh workflow enable` turns them back on. What follows is what each does when on.

Hosted CI is the cheap, fast signal and the full Windows suite runs on the
owner's machine: the
[owner's decision of 2026-09-23](../direction/Project_Context.md#hosted-ci-is-linux-per-pull-request-the-windows-suite-runs-on-each-tip-of-main-2026-09-23)
(#170). Running the whole
workspace on hosted Windows for every push had cost $124.47 of $151.65 in
three days (2026-09-20 to 09-23, from the billing API) and hit the spending
limit, which stopped every hosted job for seventeen hours; the header of
`.github/workflows/ci.yml` records it. Do not move the full suite back.

| Workflow | When | What |
|---|---|---|
| `ci.yml`, Linux job | every push to a non-draft pull request; every push to `main`; on request | `cargo fmt --all --check`, provenance, encoding, clippy with `--all-targets`, `just fmt` and `just lint`, `cargo test --workspace` with `or-renderer` under lavapipe, the BC7 encoder, the self-check, `or-io` built for `wasm32-unknown-unknown`, the skip ledger. A pull request touching only `docs/` and Markdown runs the lints and the tests that read documents instead |
| `local-gate.yml`, the `layouts` job | every push to a pull request from this repository, drafts included; every push to `main`; on request | `or-io-verify`'s per-format gates on the corpus, the behaviour baselines, then the workspace with no schema and no corpus (the metadata corpus aside) |
| `main-tip.yml` | every push to `main`; on request | `cargo check --workspace --all-targets`, the workspace tests with no corpus, clippy, the BC7 encoder, the renderer's GPU tests, `cargo hack` over each feature, the shader tool |
| `ci.yml`, Windows job | weekly, and on request | the full Windows suite on a clean hosted machine |
| `ci.yml`, macOS job | weekly, and on request | clippy, the workspace tests, the BC7 encoder and the self-check on an Apple-silicon hosted machine |
| `release.yml` | a tag starting with `v`; on request, the packages with no release | `cargo xtask dist` on each OS: a Windows `.zip`, a Linux `.tar.gz` and `.deb` and a macOS `.dmg`, each with its checksum and the `or-shader-tool` helper beside the editor (left out with a warning where its C++ will not build, unless `--require-helper`), attached to a GitHub release |

- Nothing waits for any of it before a merge. `layouts` is the only automatic
  check with the corpus, which is not on GitHub, so it is the
  only one that sees a layout disagree with real files. AGENTS.md states the
  standing CI rules.
- ⚠ The runner, `openrage-local-win` (labels `self-hosted` and
  `openrage-corpus`), is a scheduled task at logon: after a reboot it is
  absent until the owner logs in, and pull requests go without `layouts`
  until then. Its jobs share the desktop with the lanes.
- The runner's target directories live under the runner's own directory
  (the repository variable `OPENRAGE_RUNNER_ROOT`), outside the lane root, so the janitor never sweeps them. `layouts` does not run for
  a pull request from a fork, and no self-hosted job uploads an artefact: the
  corpus never leaves the machine.
- ⚠ A link failure in `main-tip` or `layouts`, `rust-lld` reporting an
  undefined symbol ending in `.llvm.` and a hash, has two causes. One is the
  default `-j`, which every self-hosted job's `CARGO_BUILD_JOBS: 4` prevents.
  The other is stale incremental state in the persistent target directories,
  `_cargo-target` (`layouts`) and
  `_cargo-target-tip` (`main-tip`) in the runner's directory, which build
  incrementally because `.cargo/config.toml` turns incremental on; a clean
  build of the same step links. The owner keeps incremental and cleans by
  hand ([2026-10-01](../direction/Project_Context.md#the-runners-stale-incremental-state-is-cleaned-by-hand-2026-10-01)):
  when no job is running, delete `debug\incremental` in both directories and
  rerun the failed job.

## Pushing and merging

A pull request is squash-merged as soon as `just ready` passes (with that
format's `just test-format MODULE` for a format's read or write path),
without waiting for CI (owner decisions, 2026-09-25, 2026-09-28 and
2026-11-20).
Merge with `gh pr merge <N> --squash --delete-branch`, then delete the branch
as [above](#deleting-a-branch-once-it-has-landed) and run the janitor.

- Push once, when the pull request is ready. Every push starts a hosted run
  and a `layouts` run, and the count of runs is what the hosting bill tracks.
- A pull request that conflicts is landed by squashing it locally onto
  `main`, with `(#N)` ending the subject, and then closed; the janitor
  recognises that landing by the `(#N)`. Never push a conflict resolution to
  a head branch.
- ⚠ GitHub retargets a pull request stacked on another branch only when that
  base branch is deleted at merge. A stacked pull request whose base survives
  looks mergeable and never reaches `main`: retarget it, or merge its base
  with `--delete-branch`.

## Running several lanes at once

The coordinator routes work to agents and combines what comes back; it does
not implement. It merges pull requests, pushes tags and carries out
recommended follow-ups without asking, and a lane may merge its own pull
request once its local checks pass. It asks the owner only about product
direction, risk appetite, or anything destructive: a force-push, a history
rewrite, deleting a branch whose work has not landed, or deleting data.

Work that needs the corpus, a game install or the GPU runs here, in lanes. A
cloud agent has no corpus and no GPU, suits work that
needs only the source and GitHub, and must fetch before reasoning about
branch state. The worker here starts with `agent worker start`, not
`cursor worker start`; a coordinator running on it dies with it, so what must
survive a restart goes in a commit or the status notes.

### How many lanes at once

The owner's figures (OD-CONCURRENCY, 2026-09-26): at most **three lanes
compile at once**, and at most five lanes holding a built `target/` stay open.
They are sized for this machine as it is set up: 8 logical CPUs, a 25%
per-tree CPU cap (`just limits-cap 25`, read from
`target/agent-limits/cap` in the checkout) and the all-agent memory ceiling
above, beside the self-hosted runner's own builds.

- A lane counts as compiling while cargo, rustc or a linker runs in it, and
  as open while its `target/` exists. Count both before starting a build or
  making a lane, and wait rather than exceed either.
- A merged lane runs `cargo clean` in its own worktree straight away, which
  frees its build output before the janitor's hour is up; run the janitor
  between waves of work.
- Merges into `main` are serial, in the order the coordinator lists.
- A format lane edits only its own Owned cell in the
  [R66d table](../direction/Roadmap.md#r66d-declare-gta-v-format-by-format), which
  `the_owned_column_is_the_resources_or_io_declares` holds to the roots
  `or-io` owns; the rest of the status text is written by one documentation
  lane after the merges.
- A lane that needs a decision the owner has not recorded stops and asks; it
  never decides. Work starts only once the decisions it depends on are on
  `origin/main`.
- Re-measure the figures with `just limits-status` after the first wave and
  record the result here.

### What every brief says

- Its own lane, made with `just agent-worktree NAME` from the checkout;
  `CARGO_TARGET_DIR` left unset; nothing outside the lane root or inside
  another project; its own pull request; no force-push or history rewrite;
  explicit paths staged, never `git add -A`.
- Comments state only what the code cannot, and a public doc comment is one
  line (the Comments section of AGENTS.md).
- An `or-io` declaration follows D15: engine names from the GTA source,
  computed offsets, no unknown fields, no provenance attribute
  ([OR_IO_Specification.md](../direction/OR_IO_Specification.md#owner-decisions)).
- A new test is proven able to fail before it is trusted; a violation is a
  finding, never a check to soften; the report says what the change catches
  and what it misses.
- A wrong layout is worse than a missing one, so anything that weakens a
  layout guard says what replaces it.
- Instructions and paths rather than pasted content, under a short imperative
  name.

### Keeping lanes apart

- ⚠ One lane, one agent. Two agents in one worktree share one index, so
  whoever commits next commits the other's staged work under their own name,
  and `git status` shows nothing wrong. Count agents against
  `git worktree list` before every dispatch.
- Messaging a finished agent restarts it in its lane, a "you are done"
  message included: release an agent by not writing to it, and address the
  agent that is live, not the owner a task board names. Before staffing a
  task, check whether it was already sent; the messages are the record.
- Do not kill a `cargo` process you cannot attribute: concurrent builds look
  alike, and a wrong kill costs another lane its build. A whole-workspace run
  of your own competes with every lane and the runner.
- Confirm an agent's first output names its lane as
  `git rev-parse --show-toplevel`; an agent that skipped that check is in the
  checkout until shown otherwise. One that says it worked around a block has
  usually edited outside its task: read its `git status`.
- A clean merge is not a compiling merge: one lane changes a signature,
  another adds a call site, and git sees no conflict. Lanes rebase onto
  `origin/main` before pushing, and `main-tip.yml` compiles every tip with
  `--all-targets`. A merge that compiles can still be wrong: a test
  generalised over a filtered list asserts the list is non-empty in the same
  commit, and an exhaustive `match` with no `_` arm is what turns one lane's
  new variant into another lane's compile error.
- A process outlives the tree that started it: a test binary still running
  after its fix merged holds file locks that fail the next link with an
  unrelated-looking error.
- A struct gaining a field breaks its literals everywhere; the lane adding the
  field fixes every literal in the workspace and says so in its pull request.
- A reproduction is evidence about a tree, so ask which one. `git log --all`
  with `-S` and `git branch --contains` answer whether any branch already
  fixes it.
- Re-measure an item before assigning it: an item already done reads exactly
  like work, and a document's claim is checked against the code before work
  is scoped from it. Strike a closed item with the reason, and check whether
  the remedy a task names has already landed before performing it.

### Evidence worth asking for

The general rules are in
[Documentation_Standard.md](Documentation_Standard.md#evidence); these are the
ones a brief most often needs.

- A cross-check counts only if its two sides can disagree; a description
  generated from the declaration it is checked against agrees by construction, and a
  differential that supplies the thing under test is not one.
- A new invariant runs against the corpus before either outcome is believed:
  a vacuous one and a false one look alike on synthetic input. A refuted one
  stays as a negative result.
- A zero needs a positive control on the same walk, and "is this field
  dropped" needs both sides of a round trip: probe, save, reload, probe.
- A test visits a parameter at a value where getting it wrong would show: a
  bit range away from bit 0, a decoy where an off-by-an-offset read lands.
  When the corpus only holds the degenerate value, supply a synthetic one.
- A test spanning strict and lenient read paths asserts on shape (counts,
  names, byte totals), never on `is_ok()`: a lenient path returns `Ok` and
  empty.
- A count travels with its denominator, and a capped list is not a total.
  Prefer a generated assertion to a written-down number: recompute and assert
  the relationship, as `src/shell_budget.rs` does.
- The first unresolved name in a bind error is a lower bound on the
  difference; diff the field sets before costing work. A grep for an
  attribute also matches the prose quoting it, so it is a floor and the
  compiler decides.
- Key a check on the question, not on today's symptom of it, and when a
  printed report and an enforcing mechanism disagree, the mechanism wins. A
  note explaining why something cannot be done is a measurement with a date.
- A gate fails on what the committer did and reports what the world did.
  Measure a probe's idle floor rather than assuming 1.0, A/B against the
  parent commit when the machine will not go quiet, and prefer a
  deterministic instrument (bytes copied against bytes kept) to a timing.
- A negative result stays as a test that fails when the conclusion changes.

### Keeping status

The coordinator's status notes, `notes.md`, live outside the repository: one
line per item saying where it stands and what is next, rewritten rather than
appended, the three newest completed items last and older ones moved to
`archived.md`. The notes are the record: a ruling that exists only in a
message is not one, and the next agent is right to refuse to act on it. What
the owner will open goes in `docs/`, in the folder of its kind, and is linked; surveys written for other
agents go in `internal/` beside the notes and are never linked.

## Known machine traps

- ⚠ A relative path handed to a .NET method from PowerShell, such as
  `[System.IO.File]::ReadAllText`, resolves against the process's working
  directory, which does not follow `cd` and starts in the checkout: from a
  lane it silently reads or writes the owner's working copy. Use absolute
  paths, or cmdlets, which follow the location.
- In a worktree `.git` is a file; put scratch such as a commit message under
  `$env:TEMP`. The stash stack is shared by every worktree, so commit work in
  progress rather than stash it.
- Edit source with an editor tool, not PowerShell `-replace`: a backtick in
  the replacement reads as an escape, and a half-working scripted edit looks
  like one that worked.
- `just janitor` from a PowerShell 7 shell fails; see
  [Cleaning up](#cleaning-up-the-janitor).
- `openrage-cli formats` stops classifying an install at a byte budget, even
  with `--all`; `--budget-gib` moves it and `--no-byte-budget` removes it, and
  an unbounded sweep can exhaust the machine.
  Scope a census with `--ext`, and never detach one you will not reap.
- If the worker's exec daemon dies on start, its bundled Node runtime does not
  match its native module: rebuild `better-sqlite3` in the worker's version
  directory.
- ⚠ A test that panics ends its process as a crash on Windows, because
  Cranelift cannot unwind, and Windows Error Reporting then keeps the dead
  process and the executable it ran from: the next build of that test binary
  fails to link with `permission denied`. A recipe that goes through
  `cargo xtask` turns the reporting off for what it starts; a test run by
  cargo or nextest directly does not. Rename the locked executable aside and
  build again.
- ⚠ A cargo started by `cargo xtask` once inherited `CARGO_MANIFEST_DIR` from
  it, and `ring`'s build script, which tracks that variable, then rebuilt
  with `rustls`, `ureq`, `or-cli` and the app, 34 to 65 s, every time a build
  alternated between a recipe and cargo run by hand. `xtask` now forgets those
  variables as it starts; the symptom, if it returns, is
  `Dirty ring: the env variable CARGO_MANIFEST_DIR changed` under `cargo -v`.
- A package selection (`-p X`) builds the features the whole workspace builds
  (`resolver.feature-unification` in `.cargo/config.toml`), so a scoped run
  and a workspace run share one set of artifacts.
