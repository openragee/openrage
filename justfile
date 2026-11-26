# OpenRage development commands. `just --list` shows each recipe with the
# comment line directly above it, so that line is the recipe's summary.

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

# A recipe is a call to `cargo` or to `cargo xtask`, which parses the same under
# `sh` and under PowerShell, so no recipe needs a shell of its own: the logic
# lives in `crates/tools/xtask`, in Rust. The recipes that still run a
# PowerShell script are `[windows]` and say so.

# An agent lane is a worktree `agent-worktree` made and marked.
lane := path_exists(justfile_directory() / ".agent-worktree")

[windows]
[private]
in-the-checkout what:
    @if ('{{ lane }}' -eq 'true') { Write-Host "``just {{ what }}`` reasons about the lane root, which is only locatable from the checkout. Run it from the checkout (wherever this repository is cloned), not from a lane." -ForegroundColor Red; exit 1 }

# ⚠ No `CARGO_TARGET_DIR` and no `CARGO_INCREMENTAL` is exported, in a lane or
# out of it. sccache hashes every `CARGO_*` variable into its key, so a per-lane
# `CARGO_TARGET_DIR` made every lane miss every other lane's entries for every
# dependency (0 of 10 hits for `png` into a second target directory, 10 of 10
# with `--target-dir`); and sccache aborts on `CARGO_INCREMENTAL=1`, whereas
# `incremental = true` in `.cargo/config.toml` reaches rustc as a flag it
# passes through uncached. A lane's `target/` is cargo's default under the
# worktree, and the runner's ambient `CARGO_TARGET_DIR` passes through untouched.

# A lane has no debugger and a panic still names its file and line, so it links
# without a PDB: the app test executable's PDB is 33 MB instead of 226 MB, and an
# edit to `or-io` rebuilds in 30 s instead of 56 s (measured 2026-10-08, with
# incremental). Outside a lane the value is `[profile.dev] debug` in Cargo.toml,
# so the checkout keeps symbolised backtraces.
export CARGO_PROFILE_DEV_DEBUG := if lane == "true" { "0" } else { env_var_or_default("CARGO_PROFILE_DEV_DEBUG", "line-tables-only") }

# sccache caches the registry crates across lanes and passes the workspace
# crates, which `incremental = true` compiles incrementally, straight through.
# An ambient wrapper is forced off elsewhere, and naming an absent wrapper fails
# every build, so an absent sccache yields "".
# ⚠ Not `which()`: it is unstable, and just 1.58 refuses it without `set lists`.
export RUSTC_WRAPPER := if lane != "true" { "" } else if os_family() == "windows" { `(Get-Command sccache -ErrorAction SilentlyContinue).Source` } else { `command -v sccache || true` }

# The test tiers write scratch under `TEMP`, which only Windows sets.
export TEMP := env_var_or_default("TEMP", env_var_or_default("TMPDIR", "/tmp"))

# List the recipes.
default:
    @just --list

# ⚠ Scoped twice. To the packages whose files differ from the merge base with
# `origin/main` and the packages that depend on them; and to the default test
# tier, since `.config/nextest.toml` names a slow tier only `just test` runs.
# Each test binary runs once with its tests as threads, not a process per test.
# The checks before a merge, over what the change touches: clippy and the default test tier; the pre-commit hook already ran fmt, encoding and provenance.
ready:
    @cargo xtask gate

# `ready` over every package, whatever changed.
ready-full:
    @cargo xtask gate --workspace

# The default test tier of what the change touches, with no lint: the loop while editing. ARGS are parts of test names to keep to.
test-affected *ARGS:
    @cargo xtask gate --tests-only -- {{ ARGS }}

# ⚠ It reads `target/nextest/all/junit.xml`, so run `just test` first, on a
# quiet machine: a module is slow by what its tests cost there.
# Rewrite the slow tier in `.config/nextest.toml` from the timings of the last `just test`.
slow-tier *ARGS:
    @cargo xtask slow-tier {{ ARGS }}

# Run the editor. ⚠ Opens a window and takes the desktop's focus: the owner's recipe, never an agent's.
dev:
    cargo run --package openrage --bin openrage

# Run the editor as `dev` does, built with LLVM and the interface's crates optimised: a frame is about a tenth of `dev`'s, the first build a few minutes. ⚠ Opens a window: the owner's recipe.
dev-fast:
    cargo run --profile editor --package openrage --bin openrage

# Draw every panel, workspace and window through real egui frames with no window; `--viewport` adds a surfaceless wgpu device.
self-check *ARGS:
    cargo run --package openrage --bin openrage -- --self-check {{ARGS}}

# Draw the shell from a script of steps and save each `shot` as a PNG in OUT, with no window; needs a GPU adapter.
shots SCRIPT OUT:
    cargo run --package openrage --bin openrage -- --self-check --shots {{quote(SCRIPT)}} {{quote(OUT)}}

# Release build of the workspace.
build:
    cargo build --release --workspace

# ⚠ nextest runs no doctests, hence the second line; the third reports what
# the run skipped, since a skip reads as a pass.
# Every test of the workspace under nextest, the slow tier included, then the doctests and the skip ledger.
test: require-nextest
    cargo nextest run --workspace --profile all
    cargo test --workspace --doc
    cargo xtask skip-ledger

# ⚠ Refuses rather than falling back to `cargo test`: `test-lite` folds its
# excluded binaries into the ledger through `nextest list`.
[private]
require-nextest:
    @cargo xtask require-nextest

# ⚠ Built with `--workspace`, not run with `cargo run -p or-test-support`:
# `-p` resolves features for that package alone, so after a workspace run it
# rebuilt `or-io` and its dependencies (114 s against 14 s).
# Drain the skip ledger, or report it with `--keep`.
[private]
skip-ledger *ARGS:
    @cargo xtask skip-ledger {{ ARGS }}

# `just test` without nextest: the same tests, about twice the wall time.
test-cargo:
    cargo test --workspace
    cargo xtask skip-ledger

# ⚠ The ledger runs whatever the tests returned and the recipe exits with the
# tests' code: it must not turn a red test green.
# The workspace suites, with a missing corpus failing instead of skipping.
test-strict:
    @cargo xtask strict-tests

# ⚠ Both variables are needed, and the condition is stated in `xtask`, which
# sets them. `the_clean_checkout_condition.rs` holds it to the `layouts` job's
# no-pack step.
# The workspace suites as a clean checkout sees them: no schema snapshot, no corpus.
test-no-pack: require-nextest
    @cargo xtask no-pack-tests

# The last run's skip report again, running nothing.
skips:
    @cargo xtask skip-ledger --keep

# One crate's library tests of the default tier, a process per test.
test-crate crate: require-nextest
    cargo nextest run -p {{ crate }} --lib

# ⚠ The cost test for `module` is `what_an_owned_<module>_costs`, and its
# timings are the dev profile's; pass `--cargo-profile release` for the
# figures a document quotes.
# One format's or-io-verify corpus suite and its owned-cost test, ignored tests included.
test-format module *ARGS: require-nextest
    cargo nextest run -p or-io-verify --profile all --test {{ module }} --test what_an_owned_file_costs --run-ignored all --success-output final -E 'binary({{ module }}) + (binary(what_an_owned_file_costs) & test(=what_an_owned_{{ module }}_costs))' {{ ARGS }}

# ⚠ `OPENRAGE_GAME_ROOT` names the install, set for this command alone; a suite
# reads tens of gigabytes, so run one at a time and never as a lane's gate.
# One install suite of a crate, a test at a time, with what each test measured printed.
test-install crate suite *ARGS: require-nextest
    cargo nextest run -p {{ crate }} --profile all --test {{ suite }} -j 1 --no-fail-fast --success-output final {{ ARGS }}

# One nextest expression rather than `--lib --bins`, which resolves features
# differently and so builds a second graph. ⚠ The last two lines put the
# binaries the filter left out into the ledger, which would otherwise report
# nothing skipped.
# Library and binary tests and the doctests; no integration suite.
test-lite: require-nextest
    cargo nextest run --workspace --profile all -E 'kind(lib) + kind(bin)'
    cargo test --workspace --doc
    @cargo xtask skip-ledger --lite

# ⚠ `--all-targets` is what reaches tests and `cfg(test)` modules.
# Clippy over every target, warnings as errors.
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Committed game content and source-tree paths and citations are fatal in
# every tracked file.
# Scan for committed game content and GTA source citations (the pre-commit hook runs it).
lint-provenance:
    cargo xtask lint-provenance

# The same citation patterns, plus a filter-repo expressions file if given, over every commit message and blob reachable from any ref of `repo`.
lint-provenance-history repo expressions="":
    cargo xtask lint-provenance --history {{ quote(join(invocation_directory_native(), repo)) }} {{ if expressions == "" { "" } else { "--expressions " + quote(join(invocation_directory_native(), expressions)) } }}

# UTF-8 BOMs and mojibake over the whole tree, or over `path`; the pre-commit hook checks the staged files.
lint-encoding path="":
    cargo xtask lint-encoding {{ if path == "" { "" } else { "--path " + quote(path) } }}

# The repository tasks' own tests: the chunked formatter against `cargo fmt --all`, and the encoding and provenance lints against repositories they build.
xtask-selftest:
    cargo test -p xtask

# ⚠ The store is found by walking ancestors: from a lane the parent directory
# is `openrage-lanes`, where nothing reads a snapshot.
# Build the metadata schema snapshot into the shared store in `openrage-corpus`.
snapshot psc_root:
    cargo run -q -p openrage --bin openrage-cli -- meta snapshot --psc-root {{ quote(psc_root) }}

# ⚠ This checks the files, never that GTA V loads them. It exits non-zero when a rule finds an error the project has not waived, when a build is refused or does not reload, or when the metadata a resource wrote disagrees with its pack.
# Validate a clothing project, build it into OUT and read each resource of the build back: the headless run of a CI job over staged files.
wardrobe-check project out build="":
    cargo run -q -p openrage --bin openrage-cli -- wardrobe validate {{ quote(project) }}
    cargo run -q -p openrage --bin openrage-cli -- wardrobe build {{ quote(project) }} --out {{ quote(out) }} {{ if build == "" { "" } else { "--build " + quote(build) } }}
    cargo run -q -p openrage --bin openrage-cli -- wardrobe verify {{ quote(project) }} --out {{ quote(out) }} {{ if build == "" { "" } else { "--build " + quote(build) } }}

# Which schema snapshot this worktree finds, what it holds and which generator wrote it.
snapshot-status:
    cargo run -q -p openrage --bin openrage-cli -- meta snapshot --status

# ⚠ It refuses while the code under test differs from the merge base with
# `origin/main` (sources, build scripts, toolchain, dependency versions), and
# fetches nothing. `OPENRAGE_REQUIRE_FIXTURES` makes a missing corpus
# fail, so nothing is recorded over nothing. Naming suites records only their
# kinds, so a new kind does not retake the records the others hold.
# Record R66e's behaviour baselines into `openrage-corpus/baselines` (OD-BASELINES); the only writer there.
baselines-record *suites:
    @cargo xtask baselines-record {{ suites }}

# ⚠ It refuses before writing anything if a file it would create exists or a
# line it would add is already there.
# Scaffold a format in or-io: its module, FORMATS row, Asset variant, census rows and gate suites.
scaffold-format module ext root version:
    @cargo xtask scaffold-format {{ quote(module) }} {{ quote(ext) }} {{ quote(root) }} {{ quote(version) }}

# Check every feature on its own and none of them (needs cargo-hack; main-tip runs it after a merge).
lint-features:
    cargo hack check --workspace --each-feature --no-dev-deps

# Install the pre-commit hook (formatting, encoding, provenance) and the binary it runs; again whenever `crates/tools/xtask` changes.
install-hooks:
    @cargo xtask install-hooks

# ⚠ Not `cargo fmt --all`, whose single rustfmt command line passes Windows'
# 32,766-character limit as targets are added and lane paths grow.
# Check formatting over every target, in bounded chunks.
fmt:
    @cargo xtask fmt --check

# Single-process: two target roots can reach one module, and two writers race.
# Format every target in place.
fmt-fix:
    @cargo xtask fmt

# Falsify the chunked formatter: it must reach the files `cargo fmt --all` reaches.
fmt-selftest:
    cargo test -p xtask --test tasks fmt

# `cargo check` the workspace.
check:
    cargo check --workspace

# Remove build output.
clean:
    cargo clean

# ⚠ The only supported way to make a lane: the janitor enumerates the lane
# root, and a worktree anywhere else is found only by name. Leave
# CARGO_TARGET_DIR unset in it.
# Make an agent lane at ../openrage-lanes/NAME on branch agent/NAME; run it from the checkout.
agent-worktree name:
    @cargo xtask lane-new {{ quote(name) }}

# A worktree goes only when its work has landed (a merged pull request at its
# HEAD, or a closed one whose `(#N)` subject is on main, which needs an
# authenticated gh), it is clean, it holds no ignored file the checkout lacks,
# nothing wrote to it for an hour and git has not locked it. It never deletes a
# branch. It finds the lane root from the checkout, so it runs from a lane too.
# Remove landed agent lanes and their build output.
janitor *args:
    @cargo xtask janitor --apply {{ args }}

# The janitor's report with nothing removed. Read this before running `janitor`.
janitor-dry *args:
    @cargo xtask janitor {{ args }}

# It counts the lane root, not free space, and a clone with no lane root passes
# it by having nothing.
# Fail when the lane root holds more than `budget` GiB.
disk-budget budget="45":
    @cargo xtask janitor --budget-gib {{ budget }}

# ⚠ Install from the checkout: the task runs the script where it was
# installed, and the janitor removes lanes. `-ExtraRoots RUNNER_DIR`
# includes the self-hosted runner.
# Register the on-demand task `limits-start` runs; it starts nothing, and nothing starts it at logon.
[windows]
limits-install *args: (in-the-checkout "limits-install")
    powershell.exe -NoLogo -ExecutionPolicy Bypass -File scripts/agent_limits.ps1 -Install {{ args }}

# Stop the limits watcher and unregister its task.
[windows]
limits-uninstall:
    powershell.exe -NoLogo -ExecutionPolicy Bypass -File {{ quote(justfile_directory() / "scripts" / "agent_limits.ps1") }} -Uninstall

# Turn agent limits on (Idle priority, memory ceilings, the saved CPU cap) until `limits-stop` or a restart.
[windows]
limits-start:
    powershell.exe -NoLogo -ExecutionPolicy Bypass -File {{ quote(justfile_directory() / "scripts" / "agent_limits.ps1") }} -Start

# Full speed: stop the watcher and lift every ceiling until `limits-start`.
[windows]
limits-stop:
    powershell.exe -NoLogo -ExecutionPolicy Bypass -File {{ quote(justfile_directory() / "scripts" / "agent_limits.ps1") }} -Stop

# Hard-cap each agent command's process tree, not all of them together, at `percent` of the machine; 0 removes it.
[windows]
limits-cap percent:
    powershell.exe -NoLogo -ExecutionPolicy Bypass -File {{ quote(justfile_directory() / "scripts" / "agent_limits.ps1") }} -SetCap {{ percent }}

# Is the limits watcher running, what has it enrolled, and what has it killed?
[windows]
limits-status:
    powershell.exe -NoLogo -ExecutionPolicy Bypass -File {{ quote(justfile_directory() / "scripts" / "agent_limits.ps1") }} -Status

# What the limits watcher would enrol right now, and why, touching nothing.
[windows]
limits-dry *args:
    powershell.exe -NoLogo -ExecutionPolicy Bypass -File {{ quote(justfile_directory() / "scripts" / "agent_limits.ps1") }} -DryRun {{ args }}

# Falsify the limits watcher against hog processes it launches itself; about 20 s, safe beside the live one.
[windows]
limits-selftest:
    powershell.exe -NoLogo -ExecutionPolicy Bypass -File {{ quote(justfile_directory() / "scripts" / "agent_limits.ps1") }} -SelfTest

# CodeWalker batch compare (Windows). Args: none = scan tests; `ytd` / `ytd,ybn` = extension filter; one path = that file; `a|b` = pipe-separated files
[windows]
cw-roundtrip *args:
    @powershell.exe -NoLogo -ExecutionPolicy Bypass -File tools/cw-compare/Invoke-RoundtripCompare.ps1{{ if args == "" { "" } else { " -JustTail " + quote(args) } }}

# The workspace linted for another OS's target from this one, with Zig compiling the C that its dependencies bring: `just cross-check windows` or `linux`.
cross-check os *args:
    cargo xtask cross-check {{os}} {{args}}

# The helper program the editor bakes the game's shaders with. It carries the C++ translator, so it needs a C++ compiler; a dev editor finds it through OPENRAGE_SHADER_TOOL=target/release/or-shader-tool.
build-helper:
    cargo build --release -p or-shader-tool --features native-front

# ⚠ The output is extracted from a game install, so it is game content:
# gitignored by name, never committed. Point OPENRAGE_SHADER_DEFS at it for
# the suites that need one.
# Build the shader definition database from an install or a directory of loose .fxc and .sps.
shader-defs input out="gta5-gen8.shaderdefs.json":
    cargo run --release -p or-shader-tool -- defs build --input {{ quote(input) }} --out {{ quote(out) }}

# What moved between two shader definition databases, installs or .fxc directories; non-zero on any difference.
shader-defs-diff before after:
    cargo run --release -p or-shader-tool -- defs diff {{ quote(before) }} {{ quote(after) }}

# How much of a set of real files a shader definition database accounts for, with the residue classified.
shader-defs-check db +files:
    cargo run --release -p or-shader-tool -- defs check {{ quote(db) }} {{ files }}
